//! Windows 透明无边框窗口：去掉系统边缘，并在原生缩放消息里约束地图比例。
//! 客户区始终采用 WM_NCCALCSIZE 传入的拟议矩形，不能读回旧窗口尺寸。

#[cfg(windows)]
#[derive(Clone, Copy)]
struct WindowState {
    hwnd: isize,
    previous: isize,
    aspect: Option<f32>,
    sizing: bool,
    sizing_width_axis: Option<bool>,
    start_rect: Option<[i32; 4]>,
    final_size: Option<[i32; 2]>,
}

#[cfg(windows)]
static WINDOWS: std::sync::Mutex<Vec<WindowState>> = std::sync::Mutex::new(Vec::new());

#[cfg(any(windows, test))]
pub(super) fn constrain_sizing(
    edge: usize,
    rect: [i32; 4],
    aspect: f32,
    minimum: [i32; 2],
    maximum: [i32; 2],
    width_axis: bool,
) -> [i32; 4] {
    if !(1..=8).contains(&edge) || !aspect.is_finite() || aspect <= 0.0 {
        return rect;
    }
    let [left, top, right, bottom] = rect;
    let aspect = f64::from(aspect);
    let min_width = f64::from(minimum[0]).max(f64::from(minimum[1]) * aspect);
    let max_width = f64::from(maximum[0])
        .min(f64::from(maximum[1]) * aspect)
        .max(min_width);
    let proposed_width = f64::from(right - left);
    let proposed_height = f64::from(bottom - top);
    let width = if edge == 3 || edge == 6 || ((4..=5).contains(&edge) && !width_axis) {
        proposed_height * aspect
    } else {
        proposed_width
    }
    .clamp(min_width, max_width)
    .round() as i32;
    let height = (f64::from(width) / aspect).round().max(1.0) as i32;
    let left_edge = matches!(edge, 1 | 4 | 7);
    let top_edge = matches!(edge, 3..=5);
    // 锚点只看被拖的那条边：拖左/右边时上下位置必须钉死，拖上/下边时左右位置钉死。
    // 以前在“没有竖直边”时按中心重排，导致拖右边窗口会上下乱跳。
    let new_left = if left_edge { right - width } else { left };
    let new_top = if top_edge { bottom - height } else { top };
    [new_left, new_top, new_left + width, new_top + height]
}

#[cfg(windows)]
fn window_state(hwnd: windows::Win32::Foundation::HWND) -> Option<WindowState> {
    WINDOWS
        .lock()
        .ok()?
        .iter()
        .find(|state| state.hwnd == hwnd.0 as isize)
        .copied()
}

#[cfg(windows)]
fn find_own_window(title: &str) -> Option<windows::Win32::Foundation::HWND> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId};
    let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let hwnd = FindWindowW(None, PCWSTR(title.as_ptr())).ok()?;
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (pid == std::process::id()).then_some(hwnd)
    }
}

#[cfg(windows)]
fn hit_test_code(direction: egui::ResizeDirection) -> isize {
    match direction {
        egui::ResizeDirection::North => 12,
        egui::ResizeDirection::South => 15,
        egui::ResizeDirection::East => 11,
        egui::ResizeDirection::West => 10,
        egui::ResizeDirection::NorthEast => 14,
        egui::ResizeDirection::SouthEast => 17,
        egui::ResizeDirection::NorthWest => 13,
        egui::ResizeDirection::SouthWest => 16,
    }
}

#[cfg(windows)]
fn resize_direction_for_hwnd(
    hwnd: windows::Win32::Foundation::HWND,
) -> Option<egui::ResizeDirection> {
    use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetWindowRect};
    let mut point = windows::Win32::Foundation::POINT::default();
    let mut rect = windows::Win32::Foundation::RECT::default();
    unsafe {
        GetCursorPos(&mut point).ok()?;
        GetWindowRect(hwnd, &mut rect).ok()?;
    }
    let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
    let border = (8.0 * dpi).ceil() as i32;
    let left = point.x - rect.left < border;
    let right = rect.right - point.x <= border;
    let top = point.y - rect.top < border;
    let bottom = rect.bottom - point.y <= border;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(egui::ResizeDirection::NorthWest),
        (_, true, true, _) => Some(egui::ResizeDirection::NorthEast),
        (true, _, _, true) => Some(egui::ResizeDirection::SouthWest),
        (_, true, _, true) => Some(egui::ResizeDirection::SouthEast),
        (true, _, _, _) => Some(egui::ResizeDirection::West),
        (_, true, _, _) => Some(egui::ResizeDirection::East),
        (_, _, true, _) => Some(egui::ResizeDirection::North),
        (_, _, _, true) => Some(egui::ResizeDirection::South),
        _ => None,
    }
}

#[cfg(windows)]
unsafe extern "system" fn borderless_wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::{LRESULT, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, GetWindowRect, IsZoomed, NCCALCSIZE_PARAMS, WMSZ_BOTTOM,
        WMSZ_BOTTOMLEFT, WMSZ_BOTTOMRIGHT, WMSZ_LEFT, WMSZ_RIGHT, WMSZ_TOP, WMSZ_TOPLEFT,
        WMSZ_TOPRIGHT, WM_ENTERSIZEMOVE, WM_ERASEBKGND, WM_EXITSIZEMOVE, WM_NCCALCSIZE,
        WM_NCDESTROY, WM_NCHITTEST, WM_SIZING, WNDPROC,
    };
    if msg == WM_ERASEBKGND {
        return LRESULT(1);
    }
    if msg == WM_NCDESTROY {
        let previous = window_state(hwnd).map(|state| state.previous).unwrap_or(0);
        if let Ok(mut states) = WINDOWS.lock() {
            states.retain(|state| state.hwnd != hwnd.0 as isize);
        }
        if previous != 0 {
            let proc: WNDPROC = std::mem::transmute(previous);
            return CallWindowProcW(proc, hwnd, msg, wparam, lparam);
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    if msg == WM_NCHITTEST {
        // 向系统报告边缘命中，让 Windows 原生 move-size loop 接管拖拽；
        // 只有调整客户区命中时才返回 HT*，按钮区继续交给 egui。
        if let Some(state) = window_state(hwnd) {
            if state.aspect.is_some() {
                let close_corner = {
                    let mut rect = RECT::default();
                    let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as i32;
                    let close_size = (36 * dpi + 95) / 96;
                    unsafe {
                        GetWindowRect(hwnd, &mut rect).is_ok() && {
                            let mut cursor = windows::Win32::Foundation::POINT::default();
                            use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
                            GetCursorPos(&mut cursor).is_ok()
                                && cursor.x >= rect.right - close_size
                                && cursor.y < rect.top + close_size
                        }
                    }
                };
                if !close_corner {
                    if let Some(direction) = resize_direction_for_hwnd(hwnd) {
                        return LRESULT(hit_test_code(direction));
                    }
                }
            }
        }
    }
    if msg == WM_SIZING {
        let Some(state) = window_state(hwnd).filter(|state| state.aspect.is_some()) else {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        };
        if lparam.0 != 0 {
            let edge = wparam.0;
            let aspect = state.aspect.unwrap();
            let mut rect = *(lparam.0 as *const RECT);
            let minimum = [240, 180];
            let maximum = [16384, 16384];
            let width_axis = {
                let Ok(mut states) = WINDOWS.lock() else {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                };
                let Some(live) = states.iter_mut().find(|item| item.hwnd == hwnd.0 as isize) else {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                };
                if let Some(axis) = live.sizing_width_axis {
                    axis
                } else {
                    let start =
                        live.start_rect
                            .unwrap_or([rect.left, rect.top, rect.right, rect.bottom]);
                    let width_delta = ((rect.right - rect.left) - (start[2] - start[0])).abs();
                    let height_delta = ((rect.bottom - rect.top) - (start[3] - start[1])).abs();
                    let axis = match edge as u32 {
                        WMSZ_TOP | WMSZ_BOTTOM => false,
                        WMSZ_LEFT | WMSZ_RIGHT => true,
                        WMSZ_TOPLEFT | WMSZ_TOPRIGHT | WMSZ_BOTTOMLEFT | WMSZ_BOTTOMRIGHT => {
                            width_delta as f32 >= height_delta as f32 * aspect
                        }
                        _ => true,
                    };
                    live.sizing_width_axis = Some(axis);
                    axis
                }
            };
            let constrained = constrain_sizing(
                edge,
                [rect.left, rect.top, rect.right, rect.bottom],
                aspect,
                minimum,
                maximum,
                width_axis,
            );
            rect.left = constrained[0];
            rect.top = constrained[1];
            rect.right = constrained[2];
            rect.bottom = constrained[3];
            *(lparam.0 as *mut RECT) = rect;
            return LRESULT(1);
        }
    }
    if msg == WM_NCCALCSIZE {
        // rgrc[0] 已经是本次的新窗口矩形。原样返回即可让客户区铺满窗口,
        // GetWindowRect 在此处读到的是旧矩形，会让缩放始终慢一拍。
        if wparam.0 != 0 && lparam.0 != 0 && IsZoomed(hwnd).as_bool() {
            let mut monitor = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            )
            .as_bool()
            {
                (*(lparam.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0] = monitor.rcWork;
            }
        }
        return LRESULT(0);
    }
    let state = window_state(hwnd);
    if msg == WM_ENTERSIZEMOVE || msg == WM_EXITSIZEMOVE {
        if let Ok(mut states) = WINDOWS.lock() {
            if let Some(state) = states
                .iter_mut()
                .find(|state| state.hwnd == hwnd.0 as isize)
            {
                state.sizing = msg == WM_ENTERSIZEMOVE;
                state.sizing_width_axis = None;
                if msg == WM_ENTERSIZEMOVE {
                    let mut rect = RECT::default();
                    state.start_rect = GetWindowRect(hwnd, &mut rect)
                        .ok()
                        .map(|_| [rect.left, rect.top, rect.right, rect.bottom]);
                    state.final_size = None;
                } else {
                    let mut rect = RECT::default();
                    state.final_size = GetWindowRect(hwnd, &mut rect).ok().map(|_| {
                        let dpi =
                            windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
                        [
                            ((rect.right - rect.left) as f32 / dpi).round() as i32,
                            ((rect.bottom - rect.top) as f32 / dpi).round() as i32,
                        ]
                    });
                    state.start_rect = None;
                }
            }
        }
    }
    // 不持有锁调用原窗口过程：Windows 消息可能重入。
    let result = match state.filter(|state| state.previous != 0) {
        Some(state) => {
            let previous: WNDPROC = std::mem::transmute(state.previous);
            CallWindowProcW(previous, hwnd, msg, wparam, lparam)
        }
        None => DefWindowProcW(hwnd, msg, wparam, lparam),
    };
    if msg == WM_NCDESTROY {
        if let Ok(mut states) = WINDOWS.lock() {
            states.retain(|state| state.hwnd != hwnd.0 as isize);
        }
    }
    result
}

/// 系统移动/缩放期间暂缓写配置，结束后读最终矩形。
#[cfg(windows)]
pub(super) fn is_interacting(title: &str) -> bool {
    find_own_window(title)
        .and_then(window_state)
        .is_some_and(|state| state.sizing)
}

#[cfg(windows)]
pub(super) fn set_resize_aspect(title: &str, aspect: Option<f32>) {
    let Some(hwnd) = find_own_window(title) else {
        return;
    };
    if let Ok(mut states) = WINDOWS.lock() {
        if let Some(state) = states
            .iter_mut()
            .find(|state| state.hwnd == hwnd.0 as isize)
        {
            state.aspect = aspect.filter(|value| value.is_finite() && *value > 0.0);
        }
    }
}

#[cfg(not(windows))]
pub(super) fn set_resize_aspect(_title: &str, _aspect: Option<f32>) {}

#[cfg(windows)]
pub(super) fn completed_size(title: &str) -> Option<[i32; 2]> {
    let hwnd = find_own_window(title)?;
    let mut states = WINDOWS.lock().ok()?;
    let state = states
        .iter_mut()
        .find(|state| state.hwnd == hwnd.0 as isize)?;
    state.final_size.take()
}

#[cfg(not(windows))]
pub(super) fn is_interacting(_title: &str) -> bool {
    false
}

#[cfg(not(windows))]
pub(super) fn completed_size(_title: &str) -> Option<[i32; 2]> {
    None
}

/// 去掉 DWM 边缘，并安装原生尺寸消息处理。只在创建窗口时调用。
#[cfg(windows)]
pub(super) fn prepare(title: &str, resizable: bool, aspect: Option<f32>) -> bool {
    use std::ffi::c_void;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_VISIBLE_FRAME_BORDER_THICKNESS, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWLP_WNDPROC, GWL_EXSTYLE, GWL_STYLE,
        SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_BORDER, WS_CAPTION, WS_DLGFRAME,
        WS_EX_TOOLWINDOW, WS_EX_WINDOWEDGE, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
    };
    let Some(hwnd) = find_own_window(title) else {
        return false;
    };
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let mut style =
            (style & !(WS_CAPTION.0 | WS_BORDER.0 | WS_DLGFRAME.0 | WS_SYSMENU.0)) | WS_POPUP.0;
        if resizable {
            style |= WS_THICKFRAME.0;
        } else {
            style &= !WS_THICKFRAME.0;
        }
        SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            ((ex_style & !WS_EX_WINDOWEDGE.0) | WS_EX_TOOLWINDOW.0) as isize,
        );
        let current = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
        let our_proc = borderless_wndproc as *const () as usize as isize;
        if current != our_proc {
            if let Ok(mut states) = WINDOWS.lock() {
                states.retain(|state| state.hwnd != hwnd.0 as isize);
                states.push(WindowState {
                    hwnd: hwnd.0 as isize,
                    previous: current,
                    aspect: aspect.filter(|value| value.is_finite() && *value > 0.0),
                    sizing: false,
                    sizing_width_axis: None,
                    start_rect: None,
                    final_size: None,
                });
            } else {
                return false;
            }
            SetWindowLongPtrW(hwnd, GWLP_WNDPROC, our_proc);
        } else if let Ok(mut states) = WINDOWS.lock() {
            if let Some(state) = states
                .iter_mut()
                .find(|state| state.hwnd == hwnd.0 as isize)
            {
                state.aspect = aspect.filter(|value| value.is_finite() && *value > 0.0);
            }
        }
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
        );
        // 不要关掉 DWM 的非客户区渲染（DWMNCRP_DISABLED）：那样窗口在缩放 / 激活时
        // 会退回 GDI 用经典主题画一圈 Win2000 风格的边框，看着像带标题栏的旧窗口。
        // 非客户区已经由 WM_NCCALCSIZE 全部让给客户区，这里只需要把 DWM 自己那圈
        // 描边 / 圆角关掉。
        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const c_void,
            std::mem::size_of_val(&corner) as u32,
        );
        let border = DWMWA_COLOR_NONE;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border as *const _ as *const c_void,
            std::mem::size_of_val(&border) as u32,
        );
        // Windows 11 起还可以把那条 1px 可视边框整个关掉（旧系统会返回错误，忽略即可）。
        let border_thickness: u32 = 0;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_VISIBLE_FRAME_BORDER_THICKNESS,
            &border_thickness as *const _ as *const c_void,
            std::mem::size_of_val(&border_thickness) as u32,
        );
        true
    }
}

#[cfg(not(windows))]
pub(super) fn prepare(_title: &str, _resizable: bool, _aspect: Option<f32>) -> bool {
    true
}
