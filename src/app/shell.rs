//! 主窗口外壳：顶部工具栏、独立窗口调度与 eframe 入口。
use super::*;

impl MhxyApp {
    pub(super) fn header_overflow_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let update_label = match self.update_phase {
            UpdatePhase::Checking => "正在检查…".to_string(),
            UpdatePhase::Available => format!(
                "更新到 {}",
                self.update_info
                    .as_ref()
                    .map(|info| info.version.as_str())
                    .unwrap_or("")
            ),
            UpdatePhase::Downloading => "查看下载进度".to_string(),
            UpdatePhase::Downloaded => "重启并更新".to_string(),
            UpdatePhase::Failed => "重新检查更新".to_string(),
            UpdatePhase::Idle => "检查更新".to_string(),
        };
        let update_enabled = self.update_phase != UpdatePhase::Checking;
        if ui
            .add_enabled(update_enabled, egui::Button::new(update_label))
            .clicked()
        {
            match self.update_phase {
                UpdatePhase::Available | UpdatePhase::Downloading | UpdatePhase::Downloaded => {
                    self.show_update_dialog = true;
                    self.update_window_pos =
                        Some(centered_window_position(ctx, UPDATE_WINDOW_SIZE));
                }
                _ => self.start_update_check(true),
            }
            ui.close();
        }
        if ui
            .button(if self.show_advanced_window {
                "关闭高级设置"
            } else {
                "高级设置"
            })
            .clicked()
        {
            self.show_advanced_window = !self.show_advanced_window;
            if self.show_advanced_window {
                self.advanced_window_pos =
                    Some(centered_window_position(ctx, ADVANCED_WINDOW_SIZE));
                self.applied_advanced_on_top = None;
            } else {
                self.advanced_window_pos = None;
            }
            ui.close();
        }
        if ui
            .button(if self.show_meditation_window {
                "关闭打坐计时"
            } else {
                "打坐计时"
            })
            .clicked()
        {
            self.show_meditation_window = !self.show_meditation_window;
            if self.show_meditation_window {
                self.meditation_window_pos =
                    Some(centered_window_position(ctx, MEDITATION_WINDOW_SIZE));
                self.applied_meditation_window_on_top = None;
            } else {
                self.meditation_window_pos = None;
            }
            ui.close();
        }
        if ui.button("说明").clicked() {
            let opening = !self.show_help;
            self.show_help = opening;
            if opening {
                self.help_window_pos = Some(centered_window_position(ctx, HELP_WINDOW_SIZE));
            }
            ui.close();
        }
        ui.menu_button("界面主题", |ui| {
            for (value, label) in [("system", "跟随系统"), ("light", "亮色"), ("dark", "暗色")]
            {
                if ui.radio(self.config.theme == value, label).clicked() {
                    self.config.theme = value.to_string();
                    self.persist();
                    ui.close();
                }
            }
        });
        let mut on_top = self.config.on_top;
        if ui.checkbox(&mut on_top, "窗口置顶").changed() {
            self.set_on_top(on_top, ctx);
        }
        ui.separator();
        if ui.button("精简界面").clicked() {
            self.enter_compact_mode();
            ui.close();
        }
    }
}

impl eframe::App for MhxyApp {
    /// 清屏色用全透明：默认值带 180 alpha，会在打坐悬浮窗背景全透明时
    /// 留下一层底色，窗口比绘制区域多出 1px 时就会表现为一条顶边线。
    /// 主窗口每个区域都有面板覆盖，alpha 为 0 不会露出黑色。
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // 跟随系统主题时，操作系统切换亮暗要即时生效。
        let light = theme::resolve_light(&self.config.theme, &ctx);
        if self.applied_light != Some(light) {
            theme::set_light(&ctx, light);
            self.applied_light = Some(light);
        }
        self.poll_updates();
        // 默认开启的定时检查：启动 2 秒后检查一次，之后每 30 分钟一次。
        // 有更新/正在下载时不再重复检查；测试环境不联网。
        if !cfg!(test)
            && self.config.update_enabled
            && self.started_at.elapsed() >= Duration::from_secs(2)
            && !matches!(
                self.update_phase,
                UpdatePhase::Checking
                    | UpdatePhase::Downloading
                    | UpdatePhase::Available
                    | UpdatePhase::Downloaded
            )
            && self
                .last_update_check
                .is_none_or(|last| last.elapsed() >= UPDATE_CHECK_INTERVAL)
        {
            self.start_update_check(false);
        }
        // 让按钮上的“已是最新/检查失败”提示自动消失。
        if let Some(at) = self.update_message_at {
            if at.elapsed() < UPDATE_FLASH_DURATION {
                ctx.request_repaint_after(UPDATE_FLASH_DURATION - at.elapsed());
            }
        }
        if matches!(
            self.update_phase,
            UpdatePhase::Checking | UpdatePhase::Downloading
        ) {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        // 收起侧栏后放宽最小宽度，让窗口可以继续缩小。
        let desired_min_size = if self.config.sidebar_expanded && !self.compact {
            MIN_WINDOW_SIZE_EXPANDED
        } else {
            MIN_WINDOW_SIZE_COLLAPSED
        };
        if self.applied_min_inner_size != Some(desired_min_size) {
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(desired_min_size));
            self.applied_min_inner_size = Some(desired_min_size);
        }
        // 启动首帧也把置顶状态同步给窗口，避免只依赖创建时的参数。
        if self.applied_on_top != Some(self.config.on_top) {
            apply_window_level(&ctx, self.config.on_top);
            self.applied_on_top = Some(self.config.on_top);
        }
        if self
            .config_dirty_since
            .is_some_and(|since| since.elapsed() >= Duration::from_millis(400))
        {
            self.flush_config();
        }
        self.poll_events();
        // 识别进行中按下的快捷键：结果回来后立刻补一次。
        self.flush_pending_manual_trigger();
        self.poll_hotkey();
        self.poll_meditation_hotkey();
        if self.config.meditation_enabled {
            self.meditation.tick();
            if self.meditation.state() == MeditationState::Running {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            if self
                .meditation
                .finished_elapsed()
                .is_some_and(|elapsed| elapsed < crate::meditation::FINISH_FLASH)
            {
                ctx.request_repaint_after(Duration::from_millis(80));
            }
        }
        // 精简模式下按 Esc 回到默认界面；正在录制快捷键时 Esc 只用于取消录制。
        if self.compact
            && !self.recording_hotkey
            && !self.meditation_recording_hotkey
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.compact = false;
            self.notice = Some("已退出精简模式".to_string());
        }
        if self.recording_hotkey {
            if let Some(captured) = hotkey::capture_egui(&ctx) {
                self.recording_hotkey = false;
                if captured != "Escape" {
                    self.config.ocr_hotkey = captured;
                    self.persist();
                }
            }
        }
        if self.meditation_recording_hotkey {
            if let Some(captured) = hotkey::capture_egui(&ctx) {
                self.meditation_recording_hotkey = false;
                if captured != "Escape" {
                    self.config.meditation_hotkey = captured;
                    self.persist();
                    self.meditation_hotkey_attempted = None;
                }
            }
        }
        self.sync_hotkey();
        // 打坐计时不依赖识别是否在跑，每帧都同步一次（失败会按 5 秒间隔重试）。
        self.sync_meditation_hotkey();
        // 非 Windows 或全局注册失败（含监听线程已退出）时，窗口聚焦也能用快捷键。
        let hotkey_active = self
            .hotkey_listener
            .as_ref()
            .is_some_and(|listener| listener.is_alive());
        if self.running && !hotkey_active && !self.busy {
            if let Ok(hotkey) = Hotkey::parse(&self.config.ocr_hotkey) {
                if hotkey.matches_egui(&ctx) {
                    self.manual_trigger();
                }
            }
        }
        // 定时重新扫描游戏窗口（游戏可能后开 / 关掉）。
        if (self.running || self.selected_handle.is_none())
            && self.last_window_refresh.elapsed() >= Duration::from_secs(3)
        {
            self.refresh_windows(false);
            self.last_window_refresh = Instant::now();
        }
        if (self.show_region_window || (self.running && !self.manual_mode()))
            && self.config.color_gate_enabled
        {
            let gate_interval =
                Duration::from_millis(self.config.color_gate_interval_ms.max(50) as u64);
            if self.last_gate_check.elapsed() >= gate_interval {
                self.check_gate();
                self.last_gate_check = Instant::now();
            }
            ctx.request_repaint_after(gate_interval);
        }
        if self.running && !self.manual_mode() && !self.busy {
            let interval = Duration::from_secs(self.config.interval_seconds.max(1) as u64);
            let due = self.last_scan.is_none_or(|last| last.elapsed() >= interval);
            if due {
                self.trigger_scan(false);
            }
        }
        // 即使暂停也定时唤醒：刷新窗口列表、连接测试结果和坐标更新时间。
        ctx.request_repaint_after(Duration::from_secs(1));
        if self.running || self.busy {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        if !self.compact {
            egui::Panel::top("header")
                .frame(
                    egui::Frame::new()
                        .fill(theme::surface())
                        .inner_margin(egui::Margin::symmetric(16, 12)),
                )
                .show(ui, |ui| {
                    // Below this width the title and all utilities no longer fit in one row.
                    // Keep primary controls visible and move secondary actions into a menu.
                    let narrow = ui.available_width() < 900.0;
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(if self.config.sidebar_expanded {
                                    if narrow {
                                        "收起"
                                    } else {
                                        "收起设置"
                                    }
                                } else if narrow {
                                    "设置"
                                } else {
                                    "展开设置"
                                })
                                .min_size(vec2(0.0, 34.0)),
                            )
                            .clicked()
                        {
                            self.config.sidebar_expanded = !self.config.sidebar_expanded;
                            self.persist();
                        }
                        ui.heading("梦幻西游");
                        if !narrow {
                            ui.label(egui::RichText::new("任务坐标辅助").color(theme::muted()));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(theme::primary_button(if self.running {
                                    if self.manual_mode() {
                                        "暂停监听"
                                    } else {
                                        "暂停识别"
                                    }
                                } else if self.manual_mode() {
                                    "开始监听"
                                } else {
                                    "开始识别"
                                }))
                                .clicked()
                            {
                                self.toggle_running();
                            }
                            if narrow {
                                ui.scope(|ui| {
                                    ui.spacing_mut().interact_size.y = 34.0;
                                    ui.menu_button("更多", |ui| {
                                        self.header_overflow_menu(ui, &ctx)
                                    });
                                });
                            }
                            if !narrow
                                && ui
                                    .add(header_button("打坐", self.show_meditation_window))
                                    .on_hover_text("打坐倒计时设置")
                                    .clicked()
                            {
                                self.show_meditation_window = !self.show_meditation_window;
                                if self.show_meditation_window {
                                    self.meditation_window_pos = Some(centered_window_position(
                                        &ctx,
                                        MEDITATION_WINDOW_SIZE,
                                    ));
                                    self.applied_meditation_window_on_top = None;
                                } else {
                                    self.meditation_window_pos = None;
                                }
                            }
                            if !narrow
                                && ui
                                    .add(header_button("高级设置", self.show_advanced_window))
                                    .on_hover_text("识别方式、方案、显示与合成旗")
                                    .clicked()
                            {
                                self.show_advanced_window = !self.show_advanced_window;
                                if self.show_advanced_window {
                                    self.advanced_window_pos =
                                        Some(centered_window_position(&ctx, ADVANCED_WINDOW_SIZE));
                                    self.applied_advanced_on_top = None;
                                } else {
                                    self.advanced_window_pos = None;
                                }
                            }
                            if !narrow
                                && ui
                                    .add(header_button("精简界面", false))
                                    .on_hover_text(
                                        "只显示地图，隐藏顶部按钮、状态栏和设置侧栏；按 Esc 退出",
                                    )
                                    .clicked()
                            {
                                self.enter_compact_mode();
                            }
                            let on_top = self.config.on_top;
                            if !narrow
                                && ui
                                    .add(header_button("置顶", on_top))
                                    .on_hover_text("让本窗口始终显示在游戏窗口前面")
                                    .clicked()
                            {
                                self.set_on_top(!on_top, &ctx);
                            }
                            if !narrow {
                                ui.scope(|ui| {
                                    ui.spacing_mut().interact_size.y = 34.0;
                                    ui.menu_button("界面主题", |ui| {
                                        for (value, label) in [
                                            ("system", "跟随系统"),
                                            ("light", "亮色"),
                                            ("dark", "暗色"),
                                        ] {
                                            if ui.radio(self.config.theme == value, label).clicked()
                                            {
                                                self.config.theme = value.to_string();
                                                self.persist();
                                                ui.close();
                                            }
                                        }
                                    })
                                    .response
                                    .on_hover_text("默认跟随系统，也可以固定亮色或暗色");
                                });
                            }
                            if !narrow
                                && ui
                                    .add(header_button("说明", self.show_help))
                                    .on_hover_text("使用说明")
                                    .clicked()
                            {
                                let opening = !self.show_help;
                                self.show_help = opening;
                                if opening {
                                    self.help_window_pos =
                                        Some(centered_window_position(ui.ctx(), HELP_WINDOW_SIZE));
                                }
                            }
                            // 更新按钮：无更新时是普通「检查更新」；发现更新后变成金色按钮，
                            // 点击弹出更新说明，不主动弹窗。
                            let total =
                                self.update_info.as_ref().map(|info| info.size).unwrap_or(0);
                            let downloaded = self.update_progress.load(Ordering::Relaxed);
                            let percent = if total > 0 {
                                (downloaded as f32 / total as f32 * 100.0).clamp(0.0, 100.0)
                            } else {
                                0.0
                            };
                            let flash_active = self
                                .update_message_at
                                .is_some_and(|at| at.elapsed() < UPDATE_FLASH_DURATION);
                            let flash_text = self.update_message.clone().unwrap_or_default();
                            let (text, color, enabled, highlight) = match self.update_phase {
                                UpdatePhase::Checking => {
                                    ("检查中…".to_string(), theme::muted(), false, false)
                                }
                                UpdatePhase::Available => (
                                    format!(
                                        "{} {}",
                                        if narrow { "更新" } else { "发现新版本" },
                                        self.update_info
                                            .as_ref()
                                            .map(|info| info.version.as_str())
                                            .unwrap_or("")
                                    ),
                                    theme::on_accent(),
                                    true,
                                    true,
                                ),
                                UpdatePhase::Downloading => {
                                    (format!("下载 {percent:.0}%"), theme::text(), true, false)
                                }
                                UpdatePhase::Downloaded => (
                                    if narrow {
                                        "重启更新"
                                    } else {
                                        "重启并更新"
                                    }
                                    .to_string(),
                                    theme::on_accent(),
                                    true,
                                    true,
                                ),
                                UpdatePhase::Failed if flash_active => {
                                    (flash_text, theme::red(), true, false)
                                }
                                UpdatePhase::Failed => {
                                    ("检查更新".to_string(), theme::text(), true, false)
                                }
                                UpdatePhase::Idle if flash_active => {
                                    (flash_text, theme::muted(), true, false)
                                }
                                UpdatePhase::Idle => (
                                    if narrow { "更新" } else { "检查更新" }.to_string(),
                                    theme::text(),
                                    true,
                                    false,
                                ),
                            };
                            let tooltip = match self.update_phase {
                                UpdatePhase::Available => format!(
                                    "发现新版本 {}\n点击查看更新说明",
                                    self.update_info
                                        .as_ref()
                                        .map(|info| info.version.as_str())
                                        .unwrap_or("")
                                ),
                                UpdatePhase::Downloading => "点击查看下载进度".to_string(),
                                UpdatePhase::Downloaded => "安装包已就绪，点击完成更新".to_string(),
                                UpdatePhase::Failed => self
                                    .update_error
                                    .clone()
                                    .unwrap_or_else(|| "检查更新失败".to_string()),
                                UpdatePhase::Checking => "正在检查更新".to_string(),
                                UpdatePhase::Idle => self
                                    .update_message
                                    .clone()
                                    .unwrap_or_else(|| "检查更新".to_string()),
                            };
                            let mut label = egui::RichText::new(text).color(color);
                            if highlight {
                                label = label.strong();
                            }
                            let mut button = egui::Button::new(label).min_size(vec2(0.0, 34.0));
                            if highlight {
                                button = button.fill(theme::gold());
                            }
                            if !narrow
                                && ui
                                    .add_enabled(enabled, button)
                                    .on_hover_text(tooltip)
                                    .clicked()
                            {
                                match self.update_phase {
                                    UpdatePhase::Available
                                    | UpdatePhase::Downloading
                                    | UpdatePhase::Downloaded => {
                                        self.show_update_dialog = true;
                                        self.update_window_pos = Some(centered_window_position(
                                            ui.ctx(),
                                            UPDATE_WINDOW_SIZE,
                                        ));
                                    }
                                    _ => self.start_update_check(true),
                                }
                            }
                        });
                    });
                });
        }
        if !self.compact {
            egui::Panel::bottom("status_bar")
                .frame(
                    egui::Frame::new()
                        .fill(theme::surface())
                        .inner_margin(egui::Margin::symmetric(16, 6)),
                )
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(crate::APP_VERSION).small().weak());
                        ui.separator();
                        ui.label(
                            egui::RichText::new(if self.config.ocr_mode == "remote" {
                                "远端 PP-OCR"
                            } else {
                                "本地 PP-OCRv5"
                            })
                            .small()
                            .weak(),
                        );
                        ui.separator();
                        if let Some(message) = &self.notice {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(message).small().color(theme::gold()),
                                )
                                .truncate(),
                            )
                            .on_hover_text(message);
                        } else if self.manual_mode() {
                            let hotkey = Hotkey::parse(&self.config.ocr_hotkey)
                                .map(|hotkey| hotkey.display())
                                .unwrap_or_else(|_| self.config.ocr_hotkey.clone());
                            let text = if self.running {
                                format!("手动识别 · 按 {hotkey} 触发")
                            } else {
                                format!("手动识别 · 快捷键 {hotkey}（未开始监听）")
                            };
                            ui.label(egui::RichText::new(text).small().weak());
                        } else {
                            ui.label(
                                egui::RichText::new(format!(
                                    "每 {} 秒识别",
                                    self.config.interval_seconds
                                ))
                                .small()
                                .weak(),
                            );
                        }
                        if let Some(seconds) = self.last_ocr_secs {
                            ui.separator();
                            ui.label(
                                egui::RichText::new(format!("上次耗时 {seconds:.2} 秒"))
                                    .small()
                                    .weak(),
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (text, color) = if self.save_error.is_some() {
                                ("配置保存失败", theme::red())
                            } else if self.config_dirty_since.is_some() {
                                ("正在保存…", theme::muted())
                            } else {
                                ("设置自动保存", theme::muted())
                            };
                            ui.label(egui::RichText::new(text).small().color(color))
                                .on_hover_text(self.save_error.as_deref().unwrap_or(text));
                            ui.separator();
                            ui.hyperlink_to(
                                egui::RichText::new("发布页").small(),
                                crate::update::HOME_URL,
                            )
                            .on_hover_text(crate::update::HOME_URL);
                            ui.separator();
                            ui.hyperlink_to(
                                egui::RichText::new("GitHub").small(),
                                crate::APP_REPO_URL,
                            )
                            .on_hover_text(crate::APP_REPO_URL);
                        });
                    });
                });
        }
        if self.config.sidebar_expanded && !self.compact {
            // 侧栏最多占可用宽度的一半减一点，保证右侧地图区域不会被挤压。
            let max_sidebar = (ui.available_width() - 360.0).clamp(240.0, 400.0);
            egui::Panel::left("controls")
                .resizable(true)
                .default_size(320.0f32.min(max_sidebar).max(240.0))
                .min_size(240.0f32.min(max_sidebar))
                .max_size(max_sidebar)
                .frame(
                    egui::Frame::new()
                        .fill(theme::background())
                        .inner_margin(12),
                )
                .show(ui, |ui| self.controls(ui));
        }
        let central_margin = if self.compact { 0 } else { 12 };
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::background())
                    .inner_margin(central_margin),
            )
            .show(ui, |ui| {
                let tasks: Vec<&'static str> = TASK_ORDER
                    .iter()
                    .copied()
                    .filter(|task| self.task_active(task))
                    .collect();
                if tasks.is_empty() {
                    return;
                }
                let rect = ui.available_rect_before_wrap();
                let count = tasks.len() as f32;
                for (index, task_type) in tasks.iter().enumerate() {
                    // 横向布局左右等分，纵向布局上下等分；只有一个任务时占满剩余区域。
                    let child_rect = if self.config.horizontal_layout {
                        let width = rect.width() / count;
                        Rect::from_min_size(
                            pos2(rect.min.x + width * index as f32, rect.min.y),
                            vec2(width, rect.height()),
                        )
                    } else {
                        let height = rect.height() / count;
                        Rect::from_min_size(
                            pos2(rect.min.x, rect.min.y + height * index as f32),
                            vec2(rect.width(), height),
                        )
                    };
                    let gap = if self.compact { 0.0 } else { 5.0 };
                    let mut child =
                        ui.new_child(egui::UiBuilder::new().max_rect(child_rect.shrink(gap)));
                    self.task_panel(&mut child, task_type);
                }
            });
        if self.show_update_dialog {
            let on_top = self.config.on_top;
            let mut builder = egui::ViewportBuilder::default()
                .with_title(format!("发现新版本 · rust-mhxy {}", crate::APP_VERSION))
                .with_inner_size(UPDATE_WINDOW_SIZE)
                .with_min_inner_size([400.0, 340.0]);
            if on_top {
                builder = builder.with_always_on_top();
            }
            if let Some(position) = self.update_window_pos {
                builder = builder.with_position(position);
            }
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("update_window"),
                builder,
                |ui, _class| {
                    if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
                        // 视口还没建好时先画在主窗口里（只出现一帧）。
                        egui::Window::new("发现新版本")
                            .collapsible(false)
                            .resizable(false)
                            .default_width(460.0)
                            .show(ui.ctx(), |ui| self.update_dialog_ui(ui));
                    } else {
                        if ui.ctx().input(|input| input.viewport().close_requested()) {
                            self.show_update_dialog = false;
                        }
                        if self.applied_update_on_top != Some(on_top) {
                            let level = if on_top {
                                egui::WindowLevel::AlwaysOnTop
                            } else {
                                egui::WindowLevel::Normal
                            };
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                            self.applied_update_on_top = Some(on_top);
                        }
                        egui::Frame::central_panel(ui.style())
                            .inner_margin(16)
                            .show(ui, |ui| {
                                ui.set_min_size(ui.available_size());
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    self.update_dialog_ui(ui);
                                });
                            });
                    }
                },
            );
        }
        if self.show_help {
            let on_top = self.config.on_top;
            let mut builder = egui::ViewportBuilder::default()
                .with_title("使用说明 · rust-mhxy")
                .with_inner_size(HELP_WINDOW_SIZE)
                .with_min_inner_size([420.0, 360.0]);
            if on_top {
                builder = builder.with_always_on_top();
            }
            if let Some(position) = self.help_window_pos {
                builder = builder.with_position(position);
            }
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("help_window"),
                builder,
                |ui, _class| {
                    if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
                        egui::Window::new("使用说明")
                            .collapsible(false)
                            .resizable(true)
                            .default_width(480.0)
                            .show(ui.ctx(), |ui| self.help_contents(ui));
                    } else {
                        if ui.ctx().input(|input| input.viewport().close_requested()) {
                            self.show_help = false;
                        }
                        if self.applied_help_on_top != Some(on_top) {
                            let level = if on_top {
                                egui::WindowLevel::AlwaysOnTop
                            } else {
                                egui::WindowLevel::Normal
                            };
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                            self.applied_help_on_top = Some(on_top);
                        }
                        egui::Frame::central_panel(ui.style())
                            .inner_margin(16)
                            .show(ui, |ui| {
                                ui.set_min_size(ui.available_size());
                                egui::ScrollArea::vertical().show(ui, |ui| self.help_contents(ui));
                            });
                    }
                },
            );
        }
        if self.show_advanced_window {
            let on_top = self.config.on_top;
            let mut builder = egui::ViewportBuilder::default()
                .with_title("高级设置")
                .with_inner_size(ADVANCED_WINDOW_SIZE)
                .with_min_inner_size([420.0, 400.0]);
            if on_top {
                builder = builder.with_always_on_top();
            }
            if let Some(position) = self.advanced_window_pos {
                builder = builder.with_position(position);
            }
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("advanced_window"),
                builder,
                |ui, _class| {
                    if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
                        egui::Window::new("高级设置")
                            .default_size(ADVANCED_WINDOW_SIZE)
                            .show(ui.ctx(), |ui| self.advanced_controls(ui));
                    } else {
                        if ui.ctx().input(|input| input.viewport().close_requested()) {
                            self.show_advanced_window = false;
                        }
                        if self.applied_advanced_on_top != Some(on_top) {
                            let level = if on_top {
                                egui::WindowLevel::AlwaysOnTop
                            } else {
                                egui::WindowLevel::Normal
                            };
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                            self.applied_advanced_on_top = Some(on_top);
                        }
                        egui::Frame::central_panel(ui.style())
                            .inner_margin(16)
                            .show(ui, |ui| {
                                let height = ui.available_height();
                                egui::ScrollArea::vertical()
                                    .id_salt("advanced_scroll")
                                    .auto_shrink([false, false])
                                    .max_height(height)
                                    .min_scrolled_height(height)
                                    .show(ui, |ui| {
                                        ui.set_min_width(ui.available_width());
                                        self.advanced_controls(ui);
                                    });
                            });
                    }
                },
            );
        }
        if self.show_meditation_window {
            let on_top = self.config.on_top;
            let mut builder = egui::ViewportBuilder::default()
                .with_title("打坐计时")
                .with_inner_size(MEDITATION_WINDOW_SIZE)
                .with_min_inner_size([360.0, 360.0]);
            if on_top {
                builder = builder.with_always_on_top();
            }
            if let Some(position) = self.meditation_window_pos {
                builder = builder.with_position(position);
            }
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("meditation_window"),
                builder,
                |ui, _class| {
                    if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
                        egui::Window::new("打坐计时")
                            .default_size(MEDITATION_WINDOW_SIZE)
                            .show(ui.ctx(), |ui| self.meditation_controls(ui));
                    } else {
                        if ui.ctx().input(|input| input.viewport().close_requested()) {
                            self.show_meditation_window = false;
                        }
                        if self.applied_meditation_window_on_top != Some(on_top) {
                            let level = if on_top {
                                egui::WindowLevel::AlwaysOnTop
                            } else {
                                egui::WindowLevel::Normal
                            };
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                            self.applied_meditation_window_on_top = Some(on_top);
                        }
                        egui::Frame::central_panel(ui.style())
                            .inner_margin(16)
                            .show(ui, |ui| {
                                // 滚动区占满窗口高度：窗口变矮时内容滚动，而不是被裁掉。
                                let height = ui.available_height();
                                egui::ScrollArea::vertical()
                                    .id_salt("meditation_scroll")
                                    .auto_shrink([false, false])
                                    .max_height(height)
                                    .min_scrolled_height(height)
                                    .show(ui, |ui| {
                                        ui.set_min_width(ui.available_width());
                                        self.meditation_controls(ui);
                                    });
                            });
                    }
                },
            );
        }
        if self.show_flight_flags_window {
            let mut builder = egui::ViewportBuilder::default()
                .with_title("合成旗编辑器")
                .with_inner_size(FLIGHT_FLAGS_WINDOW_SIZE)
                .with_min_inner_size([680.0, 560.0]);
            if self.config.on_top {
                builder = builder.with_always_on_top();
            }
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("flight_flags_window"),
                builder,
                |ui, _class| {
                    if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
                        egui::Window::new("合成旗编辑器")
                            .default_size(FLIGHT_FLAGS_WINDOW_SIZE)
                            .show(ui.ctx(), |ui| self.flight_flags_editor_ui(ui));
                    } else {
                        if ui.ctx().input(|input| input.viewport().close_requested()) {
                            self.show_flight_flags_window = false;
                        }
                        egui::Frame::central_panel(ui.style())
                            .inner_margin(16)
                            .show(ui, |ui| {
                                ui.set_min_size(ui.available_size());
                                self.flight_flags_editor_ui(ui);
                            });
                    }
                },
            );
        }
        if !self.show_region_window {
            self.release_preview();
        }
        if self.config.meditation_enabled {
            self.meditation_overlay(&ctx);
        }
        // 精简模式没有顶栏和状态栏，进入后给一个短暂的退出提示。
        if self.compact {
            if let Some(until) = self.compact_hint_until {
                if Instant::now() < until {
                    egui::Area::new("compact_hint".into())
                        .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -28.0))
                        .show(&ctx, |ui| {
                            egui::Frame::new()
                                .fill(Color32::from_black_alpha(200))
                                .corner_radius(6)
                                .inner_margin(egui::Margin::symmetric(12, 8))
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new("按 Esc 退出精简界面")
                                            .color(Color32::WHITE)
                                            .small(),
                                    );
                                });
                        });
                    ctx.request_repaint_after(Duration::from_millis(100));
                } else {
                    self.compact_hint_until = None;
                }
            }
        }
        if self.show_region_window {
            let on_top = self.config.on_top;
            let mut builder = egui::ViewportBuilder::default()
                .with_title("识别区域")
                .with_inner_size(REGION_WINDOW_SIZE)
                .with_min_inner_size([720.0, 600.0]);
            if on_top {
                // 主窗口置顶时取点窗口也必须置顶，否则会被主窗口整个盖住。
                builder = builder.with_always_on_top();
            }
            // 只在建立窗口时定位，之后用户拖动不会被抢回。
            if let Some(position) = self.region_window_pos {
                builder = builder.with_position(position);
            }
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("region_window"),
                builder,
                |ui, _class| {
                    if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
                        // 独立视口还没建好，先画在主窗口里（只出现在第一帧）。
                        egui::Window::new("识别区域")
                            .default_size([800.0, 640.0])
                            .show(ui.ctx(), |ui| self.region_window_ui(ui));
                    } else {
                        // 跟随「置顶」开关同步取点窗口的层级，开着置顶时它才不会被主窗口压住。
                        if self.applied_region_on_top != Some(on_top) {
                            let level = if on_top {
                                egui::WindowLevel::AlwaysOnTop
                            } else {
                                egui::WindowLevel::Normal
                            };
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                            self.applied_region_on_top = Some(on_top);
                        }
                        self.region_window_ui(ui);
                    }
                },
            );
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_config();
    }
}

fn header_button(text: &str, selected: bool) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_owned()))
        .selected(selected)
        .min_size(vec2(0.0, 34.0))
}

/// 说明 / 更新窗口在主窗口中间打开，并限制在屏幕内。
fn centered_window_position(ctx: &egui::Context, size: Vec2) -> egui::Pos2 {
    ctx.input(|input| {
        let viewport = input.viewport();
        let monitor = viewport
            .monitor_size
            .unwrap_or_else(|| Vec2::new(1920.0, 1080.0));
        let main = viewport.outer_rect;
        let x = main
            .map(|rect| rect.center().x - size.x / 2.0)
            .unwrap_or(0.0);
        let y = main
            .map(|rect| rect.center().y - size.y / 2.0)
            .unwrap_or(0.0);
        egui::pos2(
            x.clamp(0.0, (monitor.x - size.x).max(0.0)),
            y.clamp(0.0, (monitor.y - size.y).max(0.0)),
        )
    })
}
