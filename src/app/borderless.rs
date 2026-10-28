//! Windows 上无边框窗口的公共修饰：去掉系统投影、残留边框和那 1px 顶边。
//!
//! winit 给「无边框 + 可调大小」的窗口保留了一圈非客户区，背景透明时会在
//! 顶边露出一条白线；`DWMWA_BORDER_COLOR` 在 Windows 10 上又不生效。
//! 这里子类化窗口过程，让客户区正好等于窗口矩形，从根上消掉这条线。
//! 打坐悬浮窗和独立地图浮窗共用这套处理。

#[cfg(windows)]
static PREVIOUS_WNDPROCS: std::sync::Mutex<Vec<(isize, isize)>> = std::sync::Mutex::new(Vec::new());

/// 得到本窗口上一次的窗口过程：同一个窗口可能被反复尝试修饰。
#[cfg(windows)]
fn previous_wndproc(hwnd: windows::Win32::Foundation::HWND) -> isize {
    let key = hwnd.0 as isize;
    PREVIOUS_WNDPROCS
        .lock()
        .ok()
        .and_then(|list| {
            list.iter()
                .rev()
                .find(|(window, _)| *window == key)
                .map(|(_, previous)| *previous)
        })
        .unwrap_or(0)
}

#[cfg(windows)]
unsafe extern "system" fn borderless_wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::{LRESULT, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, GetWindowRect, NCCALCSIZE_PARAMS, WM_ERASEBKGND,
        WM_NCCALCSIZE, WNDPROC,
    };
    if msg == WM_ERASEBKGND {
        // 透明无边框窗口不要系统画白底，否则窗口刚创建 / 首次显示时会闪一下白色。
        return LRESULT(1);
    }
    if msg == WM_NCCALCSIZE && wparam.0 != 0 {
        let params = lparam.0 as *mut NCCALCSIZE_PARAMS;
        if !params.is_null() {
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_ok() {
                (*params).rgrc[0] = rect;
                return LRESULT(0);
            }
        }
    }
    let previous = previous_wndproc(hwnd);
    if previous == 0 {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let previous: WNDPROC = std::mem::transmute(previous);
    CallWindowProcW(previous, hwnd, msg, wparam, lparam)
}

/// 修饰无边框窗口：去掉边框样式、DWM 投影和圆角，并消掉顶部那 1px。
///
/// `resizable` 为 true 时保留 `WS_THICKFRAME`，这样自定义拖边缩放
/// （`ViewportCommand::BeginResize`）才能交给系统处理；非透明背景的
/// 打坐悬浮窗不需要它。返回是否成功找到窗口。
#[cfg(windows)]
pub(super) fn prepare(title: &str, resizable: bool) -> bool {
    use std::ffi::c_void;
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetWindowLongPtrW, GetWindowThreadProcessId, SetWindowLongPtrW, SetWindowPos,
        GWLP_WNDPROC, GWL_EXSTYLE, GWL_STYLE, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, WS_BORDER, WS_CAPTION, WS_DLGFRAME, WS_EX_TOOLWINDOW, WS_EX_WINDOWEDGE,
        WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
    };

    let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let Ok(hwnd) = FindWindowW(None, PCWSTR(title.as_ptr())) else {
            return false;
        };
        // 同名窗口可能属于上一个实例，确认是本进程再改。
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid != std::process::id() {
            return false;
        }

        // 清掉 winit 为无边框窗口留下的 title bar / 3D 边框样式；
        // 背景全透明时它们会在顶边留下一条线。
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let mut style =
            (style & !(WS_CAPTION.0 | WS_BORDER.0 | WS_DLGFRAME.0 | WS_SYSMENU.0)) | WS_POPUP.0;
        if resizable {
            // 保留可调大小，拖边缩放由 BeginResize 走系统循环。
            style |= WS_THICKFRAME.0;
        } else {
            style &= !WS_THICKFRAME.0;
        }
        SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let ex_style = (ex_style & !WS_EX_WINDOWEDGE.0) | WS_EX_TOOLWINDOW.0;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);

        // 子类化窗口过程，消掉 winit 为保留投影多留的 1px 顶边。
        let current = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
        let our_proc = borderless_wndproc as *const () as usize as isize;
        if current != our_proc {
            let previous = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, our_proc);
            let key = hwnd.0 as isize;
            if let Ok(mut list) = PREVIOUS_WNDPROCS.lock() {
                list.retain(|(window, _)| *window != key);
                list.push((key, previous));
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

        // Win11 默认还会画一圈阴影和圆角，背景全透明时看得到。
        // 这两个属性在 Win10 上会返回失败，忽略即可。
        let policy = DWMNCRP_DISABLED;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            &policy as *const _ as *const c_void,
            std::mem::size_of_val(&policy) as u32,
        );
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
        true
    }
}

/// 非 Windows 平台不需要额外修饰。
#[cfg(not(windows))]
pub(super) fn prepare(_title: &str, _resizable: bool) -> bool {
    true
}
