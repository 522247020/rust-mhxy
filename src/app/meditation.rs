//! 打坐计时悬浮窗与设置界面。
use super::*;

/// 打坐悬浮窗胶囊的最小尺寸。
const MEDITATION_MIN_SIZE: Vec2 = Vec2::new(64.0, 36.0);

impl MhxyApp {
    /// 打坐计时快捷键：按一下在 开始 / 暂停 / 继续 / 重新计时 之间切换。
    pub(super) fn poll_meditation_hotkey(&mut self) {
        let mut pressed = false;
        while self.meditation_hotkey_rx.try_recv().is_ok() {
            pressed = true;
        }
        if pressed {
            // 快捷键和原版一致：开始 / 重新计时；暂停、继续请点悬浮窗或设置里的按钮。
            self.meditation.restart();
        }
    }

    pub(super) fn sync_meditation_hotkey(&mut self) {
        if !self.config.meditation_enabled {
            self.meditation_hotkey_listener = None;
            self.meditation_hotkey_attempted = None;
            self.meditation_hotkey_error = None;
            self.meditation_hotkey_tried_at = None;
            return;
        }
        let raw = self.config.meditation_hotkey.trim().to_string();
        let changed = self.meditation_hotkey_attempted.as_deref() != Some(raw.as_str());
        if !changed {
            // 已注册就什么都不做；线程意外退出或注册失败则定期重试，
            // 免得界面停在「已注册」而实际按键无效。
            if self
                .meditation_hotkey_listener
                .as_ref()
                .is_some_and(|l| l.is_alive())
            {
                return;
            }
            if self.meditation_hotkey_listener.is_some() {
                self.log_detail("打坐快捷键监听线程已退出，重新注册");
                self.meditation_hotkey_listener = None;
                self.meditation_hotkey_error = None;
            }
            if self
                .meditation_hotkey_tried_at
                .is_some_and(|at| at.elapsed() < HOTKEY_RETRY_DELAY)
            {
                return;
            }
        } else if self
            .meditation_hotkey_tried_at
            .is_some_and(|at| at.elapsed() < HOTKEY_CHANGE_DELAY)
        {
            return;
        }
        self.meditation_hotkey_attempted = Some(raw.clone());
        self.meditation_hotkey_tried_at = Some(Instant::now());
        let (hotkey, label) = match Hotkey::parse(&raw) {
            Ok(hotkey) => (hotkey, hotkey.display()),
            Err(err) => {
                self.meditation_hotkey_error = Some(err);
                self.meditation_hotkey_listener = None;
                return;
            }
        };
        // 先释放旧的，避免同一个键重复注册。
        self.meditation_hotkey_listener = None;
        match hotkey::spawn(hotkey, next_hotkey_id(), self.meditation_hotkey_tx.clone()) {
            Ok(listener) => {
                self.meditation_hotkey_listener = Some(listener);
                self.meditation_hotkey_error = None;
                self.log_detail(format!("已注册打坐计时快捷键：{label}"));
            }
            Err(err) => {
                self.meditation_hotkey_listener = None;
                self.meditation_hotkey_error = Some(err.clone());
                self.log_detail(format!("打坐计时快捷键注册失败：{err}"));
            }
        }
    }

    /// 悬浮窗的文字、字体、胶囊尺寸和窗口尺寸。
    ///
    /// 窗口按「最宽的那一帧文字」定尺寸，倒计时数字变化时不需要重建窗口；
    /// 胶囊按当前文字收紧，多余部分是透明的。
    pub(super) fn meditation_layout(
        &self,
        ctx: &egui::Context,
    ) -> (String, FontId, Color32, Vec2, Vec2) {
        let font_px = if self.config.meditation_font_px == 0 {
            24.0
        } else {
            self.config.meditation_font_px as f32
        };
        let text = self.meditation.text(self.config.meditation_clock_format);
        let color = self.meditation_text_color();
        let family = if self.config.meditation_bold {
            // 有系统粗体就用粗体族，少了再靠叠加绘制加粗。
            egui::FontFamily::Name("bold".into())
        } else {
            egui::FontFamily::Proportional
        };
        let font = FontId::new(font_px, family);
        let padding = (font_px * 0.35).max(8.0);
        let measure = |sample: String| {
            ctx.fonts_mut(|fonts| fonts.layout_no_wrap(sample, font.clone(), color).size())
        };
        let capsule_of = |size: Vec2| {
            vec2(size.x + padding * 2.0, size.y + padding * 1.1).max(MEDITATION_MIN_SIZE)
        };
        let capsule = capsule_of(measure(text.clone()));
        // 倒计时最长的一帧就是初始时长：秒数格式是 "180"，分:秒格式是 "03:00"。
        let duration = self.meditation.duration_seconds();
        let widest = if self.config.meditation_clock_format {
            format!("{:02}:{:02}", duration / 60, duration % 60)
        } else {
            duration.to_string()
        };
        let window = capsule_of(measure(widest)).max(capsule);
        (text, font, color, capsule, window)
    }

    /// 打坐倒计时悬浮窗：无边框、可透明、可置顶、可鼠标穿透。
    pub(super) fn meditation_overlay(&mut self, ctx: &egui::Context) {
        let (text, font, color, capsule, window) = self.meditation_layout(ctx);
        // 改字号时窗口尺寸会连续变化，这里等 150ms 稳定后再重建，
        // 否则拖动滑块会每帧销毁/创建一个窗口，Windows 上非常卡。
        if self.meditation_applied_size != Some(window) {
            let now = Instant::now();
            // 尺寸还在变（拖字号滑块）就重置计时；稳定 150ms 后再换窗口。
            let ready = match self.meditation_pending_size {
                Some((pending, since)) if pending == window => {
                    since.elapsed() >= Duration::from_millis(150)
                }
                Some(_) => false,
                None => self.meditation_applied_size.is_none(),
            };
            if ready {
                self.meditation_applied_size = Some(window);
                self.meditation_pending_size = None;
                self.meditation_applied_on_top = None;
                self.meditation_applied_passthrough = None;
                self.meditation_shadow_attempts = 0;
            } else if self.meditation_pending_size.map(|(pending, _)| pending) != Some(window) {
                self.meditation_pending_size = Some((window, now));
            }
        }
        let size = self.meditation_applied_size.unwrap_or(window);
        let key = (size.x.round() as u32, size.y.round() as u32);
        let mut builder = egui::ViewportBuilder::default()
            .with_title("打坐计时")
            .with_decorations(false)
            .with_transparent(true)
            // 可调整大小的窗口会带 WS_SIZEBOX 边框，全透明时顶边会露出 1px 线。
            // 尺寸变化靠换 viewport id 重建窗口，不需要运行时可调。
            .with_resizable(false)
            .with_inner_size(size);
        if self.config.meditation_on_top {
            builder = builder.with_always_on_top();
        }
        if let Some((x, y)) = self.config.meditation_position {
            builder = builder.with_position(pos2(x, y));
        }
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of(("meditation_overlay", key)),
            builder,
            |ui, _class| {
                if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
                    return;
                }
                let ctx = ui.ctx().clone();
                // Windows 会给无边框窗口加 DWM 投影，背景全透明时只剩一圈阴影，
                // 这里把非客户区渲染关掉；窗口刚创建时可能还找不到，下一帧再试。
                #[cfg(windows)]
                if self.meditation_shadow_attempts < 30 {
                    self.meditation_shadow_attempts += 1;
                    if prepare_overlay_window() {
                        self.meditation_shadow_attempts = 30;
                    }
                }
                // Win+D（显示桌面）会把无边框悬浮窗一起最小化。如果不主动还原，
                // 悬浮窗就会消失，必须先点回主窗口它才会跟着回来。
                // eframe 在窗口不可见时仍会按固定间隔驱动 UI，所以这里能收到状态并自行恢复。
                // 只在「可见 → 最小化」边沿发一次，避免最小化期间每帧重复下发命令。
                let minimized = ctx.input(|input| input.viewport().minimized == Some(true));
                if minimized && !self.meditation_overlay_minimized {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                }
                self.meditation_overlay_minimized = minimized;
                if ctx.input(|input| input.viewport().close_requested()) {
                    self.config.meditation_enabled = false;
                    self.persist();
                    return;
                }
                if self.meditation_applied_on_top != Some(self.config.meditation_on_top) {
                    let level = if self.config.meditation_on_top {
                        egui::WindowLevel::AlwaysOnTop
                    } else {
                        egui::WindowLevel::Normal
                    };
                    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                    self.meditation_applied_on_top = Some(self.config.meditation_on_top);
                }
                if self.meditation_applied_passthrough != Some(self.config.meditation_locked) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(
                        self.config.meditation_locked,
                    ));
                    self.meditation_applied_passthrough = Some(self.config.meditation_locked);
                }
                let rect = ui.max_rect();
                let response = ui.allocate_rect(rect, Sense::click_and_drag());
                if response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if response.clicked() {
                    self.meditation.toggle();
                }
                if response.drag_stopped() {
                    if let Some(outer) = ctx.input(|input| input.viewport().outer_rect) {
                        self.config.meditation_position = Some((outer.min.x, outer.min.y));
                        self.persist();
                    }
                }
                let painter = ui.painter();
                let alpha = (self.config.meditation_opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
                // 窗口按最长文字定尺寸，每一帧的背景按当前文字收紧并居中。
                let background =
                    Rect::from_center_size(rect.center(), capsule.min(size)).expand(1.0);
                painter.rect_filled(
                    background,
                    CornerRadius::ZERO,
                    Color32::from_rgba_unmultiplied(0, 0, 0, alpha),
                );
                let center = rect.center();
                painter.text(
                    center + vec2(1.0, 1.2),
                    Align2::CENTER_CENTER,
                    &text,
                    font.clone(),
                    Color32::from_black_alpha(160),
                );
                painter.text(center, Align2::CENTER_CENTER, &text, font.clone(), color);
                if self.config.meditation_bold && !crate::has_bold_font() {
                    // 没有系统粗体时，四个方向各叠一层模拟加粗。
                    for offset in [vec2(0.7, 0.0), vec2(0.0, 0.7), vec2(0.7, 0.7)] {
                        painter.text(
                            center + offset,
                            Align2::CENTER_CENTER,
                            &text,
                            font.clone(),
                            color,
                        );
                    }
                }
            },
        );
    }

    pub(super) fn meditation_controls(&mut self, ui: &mut egui::Ui) {
        if ui
            .checkbox(&mut self.config.meditation_enabled, "启用打坐计时")
            .on_hover_text("启用后显示悬浮窗并注册全局快捷键")
            .changed()
        {
            if !self.config.meditation_enabled {
                self.meditation.reset();
                self.meditation_applied_size = None;
            }
            self.persist();
            ui.ctx().request_repaint();
        }
        ui.add_enabled_ui(self.config.meditation_enabled, |ui| {
            ui.horizontal(|ui| {
                if ui.button(self.meditation_action_label()).clicked() {
                    self.meditation.toggle();
                }
                if ui.button("重置").clicked() {
                    self.meditation.reset();
                }
            });
            let mut minutes = (self.config.meditation_duration_seconds / 60).max(1);
            if ui
                .add(egui::Slider::new(&mut minutes, 1..=180).text("时长（分钟）"))
                .changed()
            {
                self.config.meditation_duration_seconds = minutes * 60;
                self.meditation.set_duration_seconds(minutes * 60);
                self.persist();
            }
            if ui
                .checkbox(&mut self.config.meditation_on_top, "悬浮窗置顶")
                .on_hover_text("Windows 上始终生效；Linux Wayland 由桌面环境决定")
                .changed()
            {
                self.persist();
            }
            #[cfg(target_os = "linux")]
            if self.config.meditation_on_top && std::env::var_os("WAYLAND_DISPLAY").is_some() {
                ui.label(
                    egui::RichText::new(
                        "Wayland 不允许程序强制置顶，可在窗口规则里设为「保持在其他窗口之上」",
                    )
                    .small()
                    .color(theme::gold()),
                );
            }
            if ui
                .checkbox(&mut self.config.meditation_locked, "锁定（鼠标穿透）")
                .on_hover_text("锁定后悬浮窗点不到、拖不动，只能在主界面取消")
                .changed()
            {
                self.persist();
            }
            ui.horizontal(|ui| {
                ui.label("显示");
                let mut clock = self.config.meditation_clock_format;
                if ui.radio_value(&mut clock, false, "秒数").changed() {
                    self.config.meditation_clock_format = clock;
                    self.persist();
                }
                if ui.radio_value(&mut clock, true, "分:秒").changed() {
                    self.config.meditation_clock_format = clock;
                    self.persist();
                }
            });
            ui.horizontal(|ui| {
                ui.label("文字颜色");
                let mut color = Color32::from_rgb(
                    self.config.meditation_text_color[0],
                    self.config.meditation_text_color[1],
                    self.config.meditation_text_color[2],
                );
                if ui.color_edit_button_srgba(&mut color).changed() {
                    self.config.meditation_text_color = [color.r(), color.g(), color.b()];
                    self.persist();
                }
            });
            let mut opacity = self.config.meditation_opacity;
            if ui
                .add(egui::Slider::new(&mut opacity, 0.0..=1.0).text("背景不透明度"))
                .changed()
            {
                self.config.meditation_opacity = opacity;
                self.persist();
            }
            let mut font_px = self.config.meditation_font_px;
            if ui
                .add(egui::Slider::new(&mut font_px, 0..=96).text("字号（0 = 自动）"))
                .changed()
            {
                self.config.meditation_font_px = font_px;
                self.meditation_applied_size = None;
                self.persist();
            }
            if ui
                .checkbox(&mut self.config.meditation_bold, "加粗")
                .changed()
            {
                self.persist();
            }
            ui.horizontal(|ui| {
                ui.label("快捷键");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.config.meditation_hotkey)
                            .desired_width(72.0),
                    )
                    .changed()
                {
                    self.persist();
                }
                let record_label = if self.meditation_recording_hotkey {
                    "按键…"
                } else {
                    "录制"
                };
                if ui.button(record_label).clicked() {
                    self.meditation_recording_hotkey = !self.meditation_recording_hotkey;
                    if self.meditation_recording_hotkey {
                        self.recording_hotkey = false;
                    }
                }
            });
            match Hotkey::parse(&self.config.meditation_hotkey) {
                Ok(hotkey) => {
                    let registered = self
                        .meditation_hotkey_listener
                        .as_ref()
                        .is_some_and(|listener| listener.is_alive());
                    let status = if !self.config.meditation_enabled {
                        "未启用（开启后注册）".to_string()
                    } else if self.meditation_recording_hotkey {
                        "请按下要绑定的按键（Esc 取消）".to_string()
                    } else if registered {
                        "全局快捷键已注册".to_string()
                    } else if let Some(err) = &self.meditation_hotkey_error {
                        err.clone()
                    } else {
                        "正在注册…".to_string()
                    };
                    let color = if registered {
                        theme::muted()
                    } else {
                        theme::gold()
                    };
                    ui.label(
                        egui::RichText::new(format!("{} · {status}", hotkey.display()))
                            .small()
                            .color(color),
                    );
                }
                Err(err) => {
                    ui.label(egui::RichText::new(err).small().color(theme::red()));
                }
            }
        });
    }

    pub(super) fn meditation_action_label(&self) -> &'static str {
        match self.meditation.state() {
            MeditationState::Idle => "开始",
            MeditationState::Running => "暂停",
            MeditationState::Paused => "继续",
            MeditationState::Finished => "重新计时",
        }
    }

    /// 悬浮窗里的剩余时间文字颜色，到点后闪烁几秒。
    pub(super) fn meditation_text_color(&self) -> Color32 {
        let base = Color32::from_rgb(
            self.config.meditation_text_color[0],
            self.config.meditation_text_color[1],
            self.config.meditation_text_color[2],
        );
        let Some(elapsed) = self.meditation.finished_elapsed() else {
            return base;
        };
        if elapsed >= crate::meditation::FINISH_FLASH {
            return base;
        }
        if (elapsed.as_millis() / 250) % 2 == 0 {
            theme::red()
        } else {
            base
        }
    }
}

/// 悬浮窗原始窗口过程，子类化后转发用。
#[cfg(windows)]
static OVERLAY_PREVIOUS_WNDPROC: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);

/// 抵消 winit 为「无边框窗口投影」多留的 1px。
///
/// winit 在处理 `WM_NCCALCSIZE` 时执行 `rgrc[0].top += 1; bottom += 1`
/// （见 winit `event_loop.rs` 的 `MARKER_UNDECORATED_SHADOW` 分支，注释里也写了
/// 这会留下 1px 的顶边）。背景不透明时看不出来，背景全透明时这 1px 由系统绘制，
/// 表现为顶部一条白线；Windows 10 上 `DWMWA_BORDER_COLOR` 还不支持，改不掉它。
/// 这里让客户区正好等于窗口矩形，从根上消掉那条 1px。
#[cfg(windows)]
unsafe extern "system" fn overlay_wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::{LRESULT, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, GetWindowRect, NCCALCSIZE_PARAMS, WM_NCCALCSIZE, WNDPROC,
    };
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
    let previous: WNDPROC = std::mem::transmute(OVERLAY_PREVIOUS_WNDPROC.load(Ordering::Relaxed));
    CallWindowProcW(previous, hwnd, msg, wparam, lparam)
}

/// 修饰悬浮窗：去掉边框样式、DWM 投影，并消掉顶部那 1px。返回是否成功找到窗口。
#[cfg(windows)]
fn prepare_overlay_window() -> bool {
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

    let title: Vec<u16> = "打坐计时"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
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

        // winit 会给无边框窗口保留 WS_EX_WINDOWEDGE（3D 边框）和若干 frame 样式，
        // 背景全透明时会在顶边留下一条线，这里全部清掉。
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let style = (style
            & !(WS_CAPTION.0 | WS_THICKFRAME.0 | WS_BORDER.0 | WS_DLGFRAME.0 | WS_SYSMENU.0))
            | WS_POPUP.0;
        SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let ex_style = (ex_style & !WS_EX_WINDOWEDGE.0) | WS_EX_TOOLWINDOW.0;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);

        // 子类化窗口过程，消掉 winit 为保留投影多留的那 1px 顶边。
        // winit 只在创建窗口时设置一次窗口过程，这里判断后写入即可；
        // 窗口被重建（换 viewport id）时外层会把尝试次数清零，会重新挂一次。
        let current = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
        let our_proc = overlay_wndproc as *const () as usize as isize;
        if current != our_proc {
            let previous = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, our_proc);
            OVERLAY_PREVIOUS_WNDPROC.store(previous, Ordering::Relaxed);
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
