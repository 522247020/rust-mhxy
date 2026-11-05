//! 主窗口外壳：顶部工具栏、独立窗口调度与 eframe 入口。
use super::*;

impl MhxyApp {
    /// 「地图浮窗设置」窗口的内容：每个任务一行开关 + 宽度，下面是全局参数。
    pub(super) fn map_window_settings_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("地图浮窗");
            let open = self.config.map_window_enabled;
            if ui
                .selectable_label(open, if open { "已开启" } else { "已关闭" })
                .on_hover_text("点一下切换：每个任务的地图各用一个无边框浮窗")
                .clicked()
            {
                self.set_map_window(!open);
            }
        });
        ui.label(
            egui::RichText::new("一个任务一个无边框浮窗：在浮窗空白处拖动，拖边缘或四角等比缩放。")
                .small()
                .weak(),
        );
        if !self.config.map_window_enabled {
            ui.add_space(6.0);
            if ui
                .add(theme::primary_button("打开地图浮窗"))
                .on_hover_text("打开后每个任务各弹一个浮窗")
                .clicked()
            {
                self.set_map_window(true);
            }
            return;
        }

        ui.add_space(8.0);
        theme::section_label(ui, "每个任务");
        let tasks = self.active_tasks();
        if tasks.is_empty() {
            ui.label(egui::RichText::new("未勾选任务类型").small().weak());
        }
        for task in tasks {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    let open = self.map_window_open(task);
                    if ui
                        .selectable_label(open, format!("{task}浮窗"))
                        .on_hover_text(if open {
                            "点一下关闭这个任务的浮窗"
                        } else {
                            "点一下重新打开这个任务的浮窗"
                        })
                        .clicked()
                    {
                        self.set_map_window_visible(task, !open);
                    }
                    let mut width = self
                        .config
                        .map_window_layouts
                        .get(task)
                        .map(|layout| layout.width)
                        .unwrap_or(map_window::MAP_WINDOW_SIZE.x);
                    if ui
                        .add(
                            egui::DragValue::new(&mut width)
                                .range(240.0..=16384.0)
                                .suffix(" px"),
                        )
                        .on_hover_text("浮窗宽度（逻辑像素），高度按地图比例自动算")
                        .changed()
                    {
                        self.set_map_window_width(task, width);
                    }
                    egui::ComboBox::from_id_salt(("map-width", task))
                        .selected_text("比例")
                        .width(72.0)
                        .show_ui(ui, |ui| {
                            for (label, width) in map_window::MAP_WIDTH_PRESETS {
                                if ui.button(label).clicked() {
                                    self.set_map_window_width(task, width);
                                    ui.close();
                                }
                            }
                        });
                });
            });
        }

        ui.add_space(8.0);
        theme::section_label(ui, "浮窗样式");
        let mut opacity = self
            .config
            .map_window_opacity
            .clamp(map_window::MAP_WINDOW_MIN_OPACITY, 1.0);
        if ui
            .add(
                egui::Slider::new(&mut opacity, map_window::MAP_WINDOW_MIN_OPACITY..=1.0)
                    .text("浮窗不透明度"),
            )
            .on_hover_text("1 = 完全不透明；越小越能看到浮窗后面的游戏画面")
            .changed()
        {
            self.config.map_window_opacity = opacity;
            self.persist();
        }
        if ui
            .checkbox(&mut self.config.map_window_on_top, "浮窗置顶")
            .on_hover_text("让浮窗压在游戏前面；主窗口在前台时浮窗会自动让位")
            .changed()
        {
            self.persist();
        }
        if ui
            .checkbox(&mut self.config.map_window_locked, "锁定浮窗（鼠标穿透）")
            .on_hover_text("锁定后浮窗点不到、拖不动，只能回这里或主界面取消")
            .changed()
        {
            self.persist();
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add(theme::secondary_button("重置浮窗位置与大小"))
                .on_hover_text("所有任务浮窗恢复默认大小，并逐个错开摆回主窗口中间")
                .clicked()
            {
                self.reset_map_window_geometry(ui.ctx());
            }
            if ui
                .add(theme::secondary_button("退出地图浮窗模式"))
                .on_hover_text("关闭浮窗模式，地图回到主窗口")
                .clicked()
            {
                self.set_map_window(false);
            }
        });
    }

    /// 普通界面：退出精简，并关掉地图浮窗，让地图回到主窗口。
    pub(super) fn apply_normal_display(&mut self) {
        self.compact = false;
        self.compact_hint_until = None;
        if self.config.map_window_enabled {
            self.set_map_window(false);
        }
    }

    /// 精简界面：只留地图；浮窗模式和精简界面互斥。
    pub(super) fn apply_compact_display(&mut self) {
        if self.config.map_window_enabled {
            self.set_map_window(false);
        }
        self.enter_compact_mode();
    }

    /// 地图浮窗：先退出精简，再把地图交给各任务浮窗。
    pub(super) fn apply_map_window_display(&mut self) {
        self.compact = false;
        self.compact_hint_until = None;
        self.set_map_window(true);
    }

    pub(super) fn update_percent(&self) -> f32 {
        let total = self.update_info.as_ref().map(|info| info.size).unwrap_or(0);
        let downloaded = self.update_progress.load(Ordering::Relaxed);
        if total > 0 {
            (downloaded as f32 / total as f32 * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        }
    }

    /// 顶栏检查更新按钮：沿用更新各阶段文案和点击行为。
    pub(super) fn update_menu_entry(&self) -> (String, Color32, bool, bool, String) {
        let percent = self.update_percent();
        let flash_active = self
            .update_message_at
            .is_some_and(|at| at.elapsed() < UPDATE_FLASH_DURATION);
        let flash_text = self.update_message.clone().unwrap_or_default();
        let (text, color, enabled, highlight) = match self.update_phase {
            UpdatePhase::Checking => ("正在检查…".to_string(), theme::muted(), false, false),
            UpdatePhase::Available => (
                format!(
                    "发现新版本 {}",
                    self.update_info
                        .as_ref()
                        .map(|info| info.version.as_str())
                        .unwrap_or("")
                ),
                theme::gold(),
                true,
                true,
            ),
            UpdatePhase::Downloading => (format!("下载 {percent:.0}%"), theme::text(), true, false),
            UpdatePhase::Downloaded => ("重启并更新".to_string(), theme::gold(), true, true),
            UpdatePhase::Failed if flash_active => (flash_text, theme::red(), true, false),
            UpdatePhase::Failed => ("重新检查更新".to_string(), theme::text(), true, false),
            UpdatePhase::Idle if flash_active => (flash_text, theme::muted(), true, false),
            UpdatePhase::Idle => ("检查更新".to_string(), theme::text(), true, false),
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
        (text, color, enabled, highlight, tooltip)
    }

    pub(super) fn handle_update_click(&mut self, ctx: &egui::Context) {
        match self.update_phase {
            UpdatePhase::Available | UpdatePhase::Downloading | UpdatePhase::Downloaded => {
                self.show_update_dialog = true;
                self.update_window_pos = Some(centered_window_position(ctx, UPDATE_WINDOW_SIZE));
            }
            _ => self.start_update_check(true),
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
        if (self.sidebar_tab == "识别区域" || (self.running && !self.manual_mode()))
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
            // 实时模式：上一轮 OCR 一结束就继续下一轮，不再等固定间隔。
            // 留 100ms 下限只是为了不空转：没窗口 / 图色未命中时 trigger_scan
            // 不会置忙，否则会每帧刷一次日志。
            let due = if self.config.realtime {
                const MIN_GAP: Duration = Duration::from_millis(100);
                self.last_scan.is_none_or(|last| last.elapsed() >= MIN_GAP)
            } else {
                let interval = Duration::from_secs(self.config.interval_seconds.max(1) as u64);
                self.last_scan.is_none_or(|last| last.elapsed() >= interval)
            };
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
                        // 机身色（最凹陷的底）：顶栏和内容区一眼分开。
                        .fill(theme::inset())
                        .stroke(egui::Stroke::new(1.0, theme::border()))
                        .inner_margin(egui::Margin::symmetric(18, 14)),
                )
                .show(ui, |ui| {
                    let narrow = ui.available_width() < 900.0;
                    ui.horizontal(|ui| {
                        // 标题单行：主标题和副标题放同一行，不撑高顶栏。
                        ui.horizontal(|ui| {
                            ui.heading("梦幻西游");
                            ui.label(
                                egui::RichText::new("任务坐标辅助")
                                    .small()
                                    .color(theme::muted()),
                            );
                        });
                        // 展开 / 折叠紧跟在标题右侧。
                        let sidebar_label = if self.config.sidebar_expanded {
                            if narrow {
                                "收起"
                            } else {
                                "收起设置"
                            }
                        } else if narrow {
                            "侧栏"
                        } else {
                            "展开设置"
                        };
                        if ui
                            .add(theme::secondary_button(sidebar_label).min_size(vec2(0.0, 36.0)))
                            .on_hover_text("显示或隐藏设置侧栏")
                            .clicked()
                        {
                            self.config.sidebar_expanded = !self.config.sidebar_expanded;
                            self.persist();
                        }
                        ui.separator();
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = if narrow { 6.0 } else { 10.0 };
                            let full_start = if self.running {
                                if self.manual_mode() {
                                    "暂停监听"
                                } else {
                                    "暂停识别"
                                }
                            } else if self.manual_mode() {
                                "开始监听"
                            } else {
                                "开始识别"
                            };
                            let start_label = if narrow {
                                if self.running {
                                    "暂停"
                                } else {
                                    "开始"
                                }
                            } else {
                                full_start
                            };
                            if ui
                                .add(theme::primary_button(start_label))
                                .on_hover_text(full_start)
                                .clicked()
                            {
                                self.toggle_running();
                            }
                            if !narrow {
                                if self.running {
                                    theme::status_pill(ui, theme::jade(), "运行中");
                                } else {
                                    theme::status_pill(ui, theme::muted(), "已暂停");
                                }
                            }
                            // 检查更新常驻顶栏：跟开始按钮挨着，状态一眼能看见。
                            let (label, color, enabled, highlight, tooltip) =
                                self.update_menu_entry();
                            let mut text = egui::RichText::new(label).color(color);
                            if highlight {
                                text = text.strong();
                            }
                            let update_button = if highlight {
                                egui::Button::new(text).fill(theme::gold_tint())
                            } else {
                                egui::Button::new(text)
                            };
                            if ui
                                .add_enabled(enabled, update_button)
                                .on_hover_text(tooltip)
                                .clicked()
                            {
                                self.handle_update_click(&ctx);
                            }
                        });
                    });
                });
        }
        if !self.compact {
            egui::Panel::bottom("status_bar")
                .frame(
                    egui::Frame::new()
                        .fill(theme::inset())
                        .stroke(egui::Stroke::new(1.0, theme::border()))
                        .inner_margin(egui::Margin::symmetric(18, 9)),
                )
                .show(ui, |ui| {
                    // 窗口变窄（收缩）时左右两组文字会撞车：窄于 640 只留运行圆点、
                    // 版本和右边的保存状态/链接，识别方式、提示和耗时都不画。
                    let narrow = ui.available_width() < 640.0;
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            if self.running {
                                theme::jade()
                            } else {
                                theme::muted()
                            },
                            "●",
                        );
                        ui.label(
                            egui::RichText::new(crate::APP_VERSION)
                                .small()
                                .color(theme::text())
                                .strong(),
                        );
                        if narrow {
                            // 窄窗口：左组只剩圆点+版本，中间信息全省，三组靠右_to_left 推出去。
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
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
                                },
                            );
                            return;
                        }
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
                        } else if self.config.realtime {
                            ui.label(
                                egui::RichText::new("实时识别 · 上一轮完成立刻继续")
                                    .small()
                                    .weak(),
                            );
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
            // 左侧菜单栏：瘦长一条，只放切换主区的五个入口，不放任何控件。
            egui::Panel::left("controls")
                .resizable(false)
                .default_size(152.0)
                .min_size(140.0)
                .max_size(180.0)
                .frame(
                    egui::Frame::new()
                        .fill(theme::surface())
                        .stroke(egui::Stroke::new(1.0, theme::border()))
                        .inner_margin(10),
                )
                .show(ui, |ui| self.sidebar_menu_ui(ui));
        }
        let central_margin = if self.compact { 0 } else { 16 };
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::background())
                    .inner_margin(central_margin),
            )
            .show(ui, |ui| {
                // 精简界面：只要进了精简模式，主区只画地图（地图浮窗开着就是占位说明），
                // 不论当前在哪个设置页；Esc 退出精简模式后再回到之前的页面。
                if self.compact {
                    if self.config.map_window_enabled {
                        self.map_window_placeholder(ui);
                    } else {
                        self.task_area(ui, 255, true);
                    }
                    return;
                }

                // 主区页（顺序即优先级）：说明 / 打坐计时 / 识别区域 / 合成旗编辑器 / 行分割调试 / 识别方式 / 方案管理
                match self.sidebar_tab {
                    "识别区域" => {
                        self.region_page_ui(ui);
                        return;
                    }
                    "合成旗编辑器" => {
                        egui::ScrollArea::vertical()
                            .id_salt("flight_flags_page")
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width().min(640.0));
                                self.flight_flags_page_ui(ui);
                            });
                        return;
                    }
                    "行分割调试" => {
                        egui::ScrollArea::vertical()
                            .id_salt("rows_debug_page")
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width().min(640.0));
                                self.rows_debug_page_ui(ui);
                            });
                        return;
                    }
                    "识别方式" => {
                        egui::ScrollArea::vertical()
                            .id_salt("mode_page")
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width().min(640.0));
                                self.mode_page_ui(ui);
                            });
                        return;
                    }
                    "方案管理" => {
                        egui::ScrollArea::vertical()
                            .id_salt("profiles_page")
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width().min(640.0));
                                self.profiles_page_ui(ui);
                            });
                        return;
                    }
                    "日志" => {
                        egui::ScrollArea::vertical()
                            .id_salt("log_page")
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width().min(640.0));
                                self.log_page_ui(ui);
                            });
                        return;
                    }
                    _ => {}
                }

                if self.config.map_window_enabled && self.sidebar_tab == "任务地图" {
                    // 地图已经弹到独立浮窗：任务地图页就地给出说明 + 浮窗设置卡，
                    // 不再顶栏多一个菜单项。只在任务地图页出，其他页照常走。
                    egui::ScrollArea::vertical()
                        .id_salt("map_floating_page")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width().min(640.0));
                            self.map_window_placeholder(ui);
                            ui.add_space(12.0);
                            theme::card_with(theme::surface()).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                self.map_window_settings_controls(ui);
                            });
                        });
                    return;
                }
                if self.sidebar_tab == "说明" {
                    egui::ScrollArea::vertical()
                        .id_salt("help_page")
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width().min(640.0));
                            self.help_page_ui(ui);
                        });
                } else if self.sidebar_tab == "打坐计时" {
                    egui::ScrollArea::vertical()
                        .id_salt("meditation_page")
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width().min(640.0));
                            self.meditation_page_ui(ui);
                        });
                } else if self.sidebar_tab == "任务地图" {
                    self.task_area(ui, 255, true);
                } else {
                    // 设置页：主区滚动，内宽封顶 640，不让控件拉得太松也不能捅出窗口。
                    egui::ScrollArea::vertical()
                        .id_salt("settings_page")
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width().min(640.0));
                            self.settings_page_ui(ui);
                        });
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
        if self.sidebar_tab != "识别区域" {
            self.release_preview();
        }
        if self.config.meditation_enabled {
            self.meditation_overlay(&ctx);
        }
        // 地图浮窗：和主窗口同帧调度所有任务窗口，关闭模式后下一帧就不再创建。
        // 本进程有窗口在前台（主窗口、说明……）时，浮窗让出置顶，
        // 否则置顶浮窗会盖住菜单、下拉框和滑块，点了等于没点。
        self.app_window_in_foreground = ctx.input(|input| input.viewport().focused.unwrap_or(true))
            || borderless::foreground_is_ours();
        self.map_windows(&ctx);
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
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_config();
    }
}
