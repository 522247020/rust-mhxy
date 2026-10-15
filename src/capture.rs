//! 游戏窗口枚举与任务区域截图。
//!
//! Windows 下用 Win32 API 精确截取「客户区」（和 Python 版一致）；
//! 其他平台用 `xcap` 作为开发期的近似实现。
use crate::core::{task_region, TaskRegion};
use image::RgbImage;

#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub handle: i64,
    pub title: String,
}

/// 只把梦幻西游的窗口列出来，免得浏览器、编辑器这些大窗口也混进可选项。
const GAME_TITLE_KEYWORDS: &[&str] = &["梦幻西游", "MHXY"];

fn filter_game_windows(windows: Vec<WindowInfo>) -> Vec<WindowInfo> {
    let matched: Vec<WindowInfo> = windows
        .iter()
        .filter(|window| {
            GAME_TITLE_KEYWORDS
                .iter()
                .any(|keyword| window.title.contains(keyword))
        })
        .cloned()
        .collect();
    let mut result = if matched.is_empty() { windows } else { matched };
    result.sort_by_key(|window| window.title.to_lowercase());
    result
}

fn crop_region(full: &RgbImage, region: TaskRegion) -> RgbImage {
    let (width, height) = full.dimensions();
    let (left, top, right, bottom) = task_region(width as f32, height as f32, region);
    let x = left.max(0.0).min(width as f32) as u32;
    let y = top.max(0.0).min(height as f32) as u32;
    let w = (right - left).max(1.0).min(width as f32 - x as f32) as u32;
    let h = (bottom - top).max(1.0).min(height as f32 - y as f32) as u32;
    image::imageops::crop_imm(full, x, y, w, h).to_image()
}

/// 把多个截图区域纵向拼成一张，这样多区域也只需要识别一次。
fn stack_panels(panels: &[RgbImage]) -> RgbImage {
    if panels.len() == 1 {
        return panels[0].clone();
    }
    const GAP: u32 = 8;
    let width = panels.iter().map(|panel| panel.width()).max().unwrap_or(1);
    let height =
        panels.iter().map(|panel| panel.height()).sum::<u32>() + GAP * (panels.len() as u32 - 1);
    let mut combined = RgbImage::from_pixel(width, height, image::Rgb([255, 255, 255]));
    let mut offset = 0;
    for panel in panels {
        image::imageops::replace(&mut combined, panel, 0, offset as i64);
        offset += panel.height() + GAP;
    }
    combined
}

/// 截图并切出主区域（以及可选的备用区域），返回 (拼接后的面板, 完整客户区截图)。
pub fn capture_task_panel(
    handle: i64,
    region: TaskRegion,
    extra: Option<TaskRegion>,
) -> Result<(RgbImage, RgbImage), String> {
    let full = imp::capture_client(handle, region)?;
    let mut panels = vec![crop_region(&full, region)];
    if let Some(extra) = extra {
        panels.push(crop_region(&full, extra));
    }
    Ok((stack_panels(&panels), full))
}

pub fn list_windows() -> Result<Vec<WindowInfo>, String> {
    let windows = imp::list_windows()?;
    Ok(filter_game_windows(windows))
}

/// 只截取客户区里的一小块（图色门控用），开销远小于整张客户区。
pub fn capture_client_rect(
    handle: i64,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<RgbImage, String> {
    imp::capture_client_rect(handle, x, y, width.max(1), height.max(1))
}

#[cfg(not(windows))]
mod imp {
    use super::WindowInfo;
    use image::RgbImage;

    pub fn list_windows() -> Result<Vec<WindowInfo>, String> {
        let windows = xcap::Window::all().map_err(|err| format!("枚举窗口失败：{err}"))?;
        let mut items = Vec::new();
        for window in windows {
            if window.pid().ok() == Some(std::process::id()) {
                continue;
            }
            let title = window.title().unwrap_or_default().trim().to_string();
            if title.is_empty() {
                continue;
            }
            if window.is_minimized().unwrap_or(false) {
                continue;
            }
            let width = window.width().unwrap_or(0);
            let height = window.height().unwrap_or(0);
            if width < 500 || height < 350 {
                continue;
            }
            let handle = window.id().unwrap_or(0) as i64;
            items.push(WindowInfo { handle, title });
        }
        Ok(items)
    }

    pub fn capture_client(
        handle: i64,
        _region: crate::core::TaskRegion,
    ) -> Result<RgbImage, String> {
        let windows = xcap::Window::all().map_err(|err| format!("枚举窗口失败：{err}"))?;
        let window = windows
            .into_iter()
            .find(|window| window.id().map(|id| id as i64).unwrap_or(-1) == handle)
            .ok_or_else(|| "游戏窗口已关闭或最小化".to_string())?;
        if window.is_minimized().unwrap_or(false) {
            return Err("游戏窗口已关闭或最小化".into());
        }
        let rgba = window
            .capture_image()
            .map_err(|err| format!("截图失败：{err}"))?;
        let (width, height) = rgba.dimensions();
        if width < 500 || height < 350 {
            return Err(format!("游戏客户区尺寸过小：{width}×{height}"));
        }
        Ok(RgbImage::from_fn(width, height, |x, y| {
            let pixel = rgba.get_pixel(x, y);
            image::Rgb([pixel[0], pixel[1], pixel[2]])
        }))
    }

    pub fn capture_client_rect(
        handle: i64,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<RgbImage, String> {
        let full = capture_client(handle, crate::core::DEFAULT_TASK_REGION)?;
        let (full_w, full_h) = full.dimensions();
        let x = x.clamp(0, full_w as i32 - 1) as u32;
        let y = y.clamp(0, full_h as i32 - 1) as u32;
        let width = width.clamp(1, full_w as i32 - x as i32) as u32;
        let height = height.clamp(1, full_h as i32 - y as i32) as u32;
        Ok(image::imageops::crop_imm(&full, x, y, width, height).to_image())
    }
}

#[cfg(windows)]
mod imp {
    use super::WindowInfo;
    use crate::core::task_region;
    use image::RgbImage;
    use std::mem::size_of;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, TRUE};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
        GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetAncestor, GetClientRect, GetWindowRect, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, WindowFromPoint, GA_ROOT,
    };

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let windows = &mut *(lparam.0 as *mut Vec<WindowInfo>);
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return TRUE;
        }
        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        if process_id == std::process::id() {
            return TRUE;
        }
        let mut buffer = [0u16; 512];
        let length = GetWindowTextW(hwnd, &mut buffer);
        if length <= 0 {
            return TRUE;
        }
        let title = String::from_utf16_lossy(&buffer[..length as usize])
            .trim()
            .to_string();
        if title.is_empty() {
            return TRUE;
        }
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return TRUE;
        }
        if rect.right - rect.left >= 500 && rect.bottom - rect.top >= 350 {
            windows.push(WindowInfo {
                handle: hwnd.0 as i64,
                title,
            });
        }
        TRUE
    }

    pub fn list_windows() -> Result<Vec<WindowInfo>, String> {
        let mut windows: Vec<WindowInfo> = Vec::new();
        unsafe {
            let _ = EnumWindows(
                Some(enum_proc),
                LPARAM(&mut windows as *mut Vec<WindowInfo> as isize),
            );
        }
        Ok(windows)
    }

    unsafe fn capture_screen(x: i32, y: i32, width: i32, height: i32) -> Result<RgbImage, String> {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return Err("获取屏幕 DC 失败".into());
        }
        let memory_dc = CreateCompatibleDC(Some(screen_dc));
        let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
        let old = SelectObject(memory_dc, HGDIOBJ(bitmap.0));
        let result = (|| {
            BitBlt(
                memory_dc,
                0,
                0,
                width,
                height,
                Some(screen_dc),
                x,
                y,
                SRCCOPY,
            )
            .map_err(|err| format!("截图失败：{err}"))?;
            let mut info = BITMAPINFO::default();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            // 负数高度表示自上而下存储。
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB.0;
            let mut buffer = vec![0u8; (width * height * 4) as usize];
            let lines = GetDIBits(
                memory_dc,
                bitmap,
                0,
                height as u32,
                Some(buffer.as_mut_ptr() as *mut core::ffi::c_void),
                &mut info,
                DIB_RGB_COLORS,
            );
            if lines == 0 {
                return Err("读取位图失败".into());
            }
            let mut image = RgbImage::new(width as u32, height as u32);
            for (pixel, source) in image.pixels_mut().zip(buffer.chunks_exact(4)) {
                *pixel = image::Rgb([source[2], source[1], source[0]]);
            }
            Ok(image)
        })();
        SelectObject(memory_dc, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory_dc);
        ReleaseDC(None, screen_dc);
        result
    }

    pub fn capture_client(
        handle: i64,
        region: crate::core::TaskRegion,
    ) -> Result<RgbImage, String> {
        unsafe {
            let hwnd = HWND(handle as *mut core::ffi::c_void);
            if !IsWindow(Some(hwnd)).as_bool() || IsIconic(hwnd).as_bool() {
                return Err("游戏窗口已关闭或最小化".into());
            }
            let mut client = RECT::default();
            GetClientRect(hwnd, &mut client).map_err(|err| format!("读取窗口大小失败：{err}"))?;
            let width = client.right;
            let height = client.bottom;
            if width < 500 || height < 350 {
                return Err(format!("游戏客户区尺寸过小：{width}×{height}"));
            }
            let mut origin = POINT { x: 0, y: 0 };
            if !ClientToScreen(hwnd, &mut origin).as_bool() {
                return Err("定位游戏窗口失败".into());
            }
            // 任务区域被其他窗口挡住时截到的是别的画面，先检查一下。
            let (left, top, right, bottom) = task_region(width as f32, height as f32, region);
            let center = POINT {
                x: origin.x + ((left + right) / 2.0) as i32,
                y: origin.y + ((top + bottom) / 2.0) as i32,
            };
            let covering = GetAncestor(WindowFromPoint(center), GA_ROOT);
            if covering.0 != hwnd.0 {
                return Err("任务区域被其他窗口遮挡".into());
            }
            capture_screen(origin.x, origin.y, width, height)
        }
    }

    pub fn capture_client_rect(
        handle: i64,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<RgbImage, String> {
        unsafe {
            let hwnd = HWND(handle as *mut core::ffi::c_void);
            if !IsWindow(Some(hwnd)).as_bool() || IsIconic(hwnd).as_bool() {
                return Err("游戏窗口已关闭或最小化".into());
            }
            let mut origin = POINT { x: 0, y: 0 };
            if !ClientToScreen(hwnd, &mut origin).as_bool() {
                return Err("定位游戏窗口失败".into());
            }
            capture_screen(origin.x + x, origin.y + y, width, height)
        }
    }
}
