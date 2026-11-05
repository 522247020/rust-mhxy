//! 侧栏控件、配置方案与日志界面。
use super::*;

impl MhxyApp {
    /// 简单日志：启动应用、任务开始/停止。详细模式下也显示。
    pub(super) fn log(&mut self, message: impl Into<String>) {
        self.push_log(true, message.into());
    }

    /// 详细日志：只在「详细」级别显示（OCR 过程、错误、窗口变化等）。
    pub(super) fn log_detail(&mut self, message: impl Into<String>) {
        self.push_log(false, message.into());
    }

    /// 日志同时写内存环形缓冲（日志页实时看）和磁盘（logs/rust-mhxy.log）；详细级别才记录 OCR 过程。
    pub(super) fn push_log(&mut self, simple: bool, message: String) {
        if !simple && self.config.log_level != "detailed" {
            return;
        }
        let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let line = format!("[{time}] {message}");
        if self.log_lines.len() >= LOG_BUFFER_MAX {
            self.log_lines.pop_front();
        }
        self.log_lines.push_back(line.clone());
        append_log(&self.log_path, &format!("{line}\n"));
    }
    pub(super) fn profile_dirty(&self) -> bool {
        let current = Config::profile_from_current(&self.config);
        self.config
            .profiles
            .iter()
            .find(|profile| profile.name == self.config.active_profile)
            .is_none_or(|profile| *profile != current)
    }

    pub(super) fn apply_profile(&mut self, name: &str) {
        let Some(profile) = self
            .config
            .profiles
            .iter()
            .find(|profile| profile.name == name)
            .cloned()
        else {
            return;
        };
        self.config.active_profile = profile.name.clone();
        // 方案里带的识别区域：应用后同步到当前分辨率，避免下次识别又被自动记忆覆盖。
        if let Some(region) = profile.task_region {
            self.config.task_region = region;
            if let Some(size) = self
                .last_client_size
                .or_else(|| self.selected_handle.and_then(capture::client_size))
            {
                self.config
                    .region_presets
                    .insert(format!("{}x{}", size.0, size.1), region_to_string(region));
                self.last_client_size = Some(size);
            }
        }
        if let Some(region) = profile.extra_task_region {
            self.config.extra_task_region = region;
        }
        if let Some(enabled) = profile.extra_region_enabled {
            self.config.extra_region_enabled = enabled;
        }
        self.config.color_gate_enabled = profile.gate_enabled;
        self.config.color_gate_interval_ms = profile.gate_interval_ms;
        self.config.color_gate_mode = profile.gate_mode;
        self.config.color_gate_tolerance = profile.gate_tolerance;
        self.config.color_gate_points = profile.gate_points;
        self.invalidate_scan();
        self.check_gate();
        self.persist();
        self.log_detail(format!("切换识别方案：{name}"));
    }

    pub(super) fn save_active_profile(&mut self) {
        let current = Config::profile_from_current(&self.config);
        if let Some(profile) = self
            .config
            .profiles
            .iter_mut()
            .find(|profile| profile.name == self.config.active_profile)
        {
            *profile = current;
            self.persist();
            self.notice = Some(format!("已保存方案「{}」", self.config.active_profile));
        }
    }

    pub(super) fn create_profile(&mut self) {
        let mut name = self.profile_name_edit.trim().to_string();
        if name.is_empty() {
            name = format!("方案 {}", self.config.profiles.len() + 1);
        }
        let mut unique = name.clone();
        let mut index = 2;
        while self
            .config
            .profiles
            .iter()
            .any(|profile| profile.name == unique)
        {
            unique = format!("{name} {index}");
            index += 1;
        }
        self.config.active_profile = unique.clone();
        let profile = Config::profile_from_current(&self.config);
        self.config.profiles.push(profile);
        self.profile_saving = false;
        self.persist();
        self.notice = Some(format!("已新建方案「{unique}」"));
    }

    pub(super) fn delete_active_profile(&mut self) {
        if self.config.profiles.len() <= 1 {
            return;
        }
        let name = self.config.active_profile.clone();
        self.config.profiles.retain(|profile| profile.name != name);
        if let Some(first) = self
            .config
            .profiles
            .first()
            .map(|profile| profile.name.clone())
        {
            self.apply_profile(&first);
        }
        self.notice = Some(format!("已删除方案「{name}」"));
    }

    pub(super) fn profile_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("识别方案").strong());
            let dirty = self.profile_dirty();
            let label = if dirty {
                format!("{} *", self.config.active_profile)
            } else {
                self.config.active_profile.clone()
            };
            let mut chosen: Option<String> = None;
            egui::ComboBox::from_id_salt("profile_select")
                .selected_text(label)
                .width(150.0)
                .show_ui(ui, |ui| {
                    for profile in &self.config.profiles {
                        if ui
                            .selectable_label(
                                profile.name == self.config.active_profile,
                                &profile.name,
                            )
                            .clicked()
                        {
                            chosen = Some(profile.name.clone());
                        }
                    }
                });
            if let Some(name) = chosen {
                self.apply_profile(&name);
            }
            if ui
                .small_button("保存")
                .on_hover_text("把当前的图色门控设置保存到当前方案")
                .clicked()
            {
                self.save_active_profile();
            }
            if ui
                .small_button("另存为")
                .on_hover_text("用当前设置新建一个方案")
                .clicked()
            {
                self.profile_saving = true;
                self.profile_name_edit = format!("方案 {}", self.config.profiles.len() + 1);
            }
            if ui
                .add_enabled(
                    self.config.profiles.len() > 1,
                    theme::danger_button("删除").small(),
                )
                .on_hover_text("删除当前方案（至少保留一个）")
                .clicked()
            {
                self.delete_active_profile();
            }
            if dirty {
                ui.label(
                    egui::RichText::new("有未保存的修改")
                        .small()
                        .color(theme::gold()),
                );
            }
        });
        if self.profile_saving {
            ui.horizontal_wrapped(|ui| {
                ui.label("方案名称");
                ui.add(
                    egui::TextEdit::singleline(&mut self.profile_name_edit).desired_width(140.0),
                );
                if ui.button("确定").clicked() {
                    self.create_profile();
                }
                if ui.button("取消").clicked() {
                    self.profile_saving = false;
                }
            });
        }
    }
    /// 左侧菜单栏：页面切换 + 工具/设置/显示的全部入口，都在这里，不再挂顶栏「更多」。
    pub(super) fn sidebar_menu_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical()
            .id_salt("menu_bar")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                // 页面：切换主区看什么。
                theme::section_label(ui, "页面");
                for tab in [
                    "任务地图",
                    "游戏窗口",
                    "识别区域",
                    "识别模式",
                    "任务类型",
                    "显示与布局",
                    "打坐计时",
                    "说明",
                ] {
                    let active = self.sidebar_tab == tab;
                    // 未选中：纯文本行；选中：浅灰高亮（Win11 导航风），文字保持正色。
                    let label = egui::RichText::new(tab).size(14.0).color(theme::text());
                    let button = egui::Button::new(label).wrap_mode(egui::TextWrapMode::Extend);
                    let button = if active {
                        button.fill(theme::inset())
                    } else {
                        button.fill(Color32::TRANSPARENT)
                    };
                    if ui.add_sized([ui.available_width(), 34.0], button).clicked() {
                        self.sidebar_tab = tab;
                    }
                }
                theme::divider(ui);
                theme::divider(ui);
                // 显示：界面模式和主题。
                theme::section_label(ui, "显示");
                if Self::menu_toggle(
                    ui,
                    "地图浮窗",
                    self.config.map_window_enabled,
                    "每个任务一个无边框浮窗；参数在任务地图页下方的浮窗设置卡里",
                )
                .clicked()
                {
                    if self.config.map_window_enabled {
                        self.apply_normal_display();
                    } else {
                        self.apply_map_window_display();
                        // 打开后直接落到任务地图页：浮窗参数就地在占位说明下方，不另开页面。
                        self.sidebar_tab = "任务地图";
                    }
                }
                if Self::menu_toggle(
                    ui,
                    "窗口置顶",
                    self.config.on_top,
                    "让本窗口始终显示在游戏窗口前面",
                )
                .clicked()
                {
                    let on = !self.config.on_top;
                    self.set_on_top(on, &ctx);
                }
                // 界面主题：点一下循环切换跟随系统 / 亮色 / 暗色。
                let theme_label = match self.config.theme.as_str() {
                    "light" => "亮色",
                    "dark" => "暗色",
                    _ => "跟随系统",
                };
                if ui
                    .add_sized(
                        [ui.available_width(), 32.0],
                        egui::Button::new(
                            egui::RichText::new(format!("界面主题 · {theme_label}")).size(13.0),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend),
                    )
                    .on_hover_text("点一下切换：跟随系统 → 亮色 → 暗色")
                    .clicked()
                {
                    self.config.theme = match self.config.theme.as_str() {
                        "system" => "light".to_string(),
                        "light" => "dark".to_string(),
                        _ => "system".to_string(),
                    };
                    self.persist();
                }
                if Self::menu_action(ui, "精简界面")
                    .on_hover_text("只留地图；按 Esc 恢复")
                    .clicked()
                {
                    self.apply_compact_display();
                }
                theme::divider(ui);
                theme::divider(ui);
                // 高级设置：单独一个分组在侧边，合成旗编辑器 / 识别方式 / 方案管理 /
                // 行分割调试 / 日志都放里面。
                theme::section_label(ui, "高级设置");
                for tab in ["合成旗编辑器", "识别方式", "方案管理", "行分割调试", "日志"]
                {
                    let active = self.sidebar_tab == tab;
                    let label = egui::RichText::new(tab).size(14.0).color(theme::text());
                    let button = egui::Button::new(label).wrap_mode(egui::TextWrapMode::Extend);
                    let button = if active {
                        button.fill(theme::inset())
                    } else {
                        button.fill(Color32::TRANSPARENT)
                    };
                    if ui.add_sized([ui.available_width(), 34.0], button).clicked() {
                        self.sidebar_tab = tab;
                    }
                }
            });
    }

    /// 菜单里的开关项：开着时浅灰高亮，关着时纯文本行。
    fn menu_toggle(ui: &mut egui::Ui, text: &str, on: bool, tooltip: &str) -> egui::Response {
        let button = egui::Button::new(egui::RichText::new(text).size(13.0).color(theme::text()))
            .wrap_mode(egui::TextWrapMode::Extend);
        let button = if on {
            button.fill(theme::inset())
        } else {
            button.fill(Color32::TRANSPARENT)
        };
        ui.add_sized([ui.available_width(), 32.0], button)
            .on_hover_text(tooltip)
    }

    /// 菜单里的普通动作项：纯文本行，悬停浮出淡底。
    fn menu_action(ui: &mut egui::Ui, text: &str) -> egui::Response {
        ui.add_sized(
            [ui.available_width(), 32.0],
            egui::Button::new(egui::RichText::new(text).size(13.0).color(theme::text()))
                .wrap_mode(egui::TextWrapMode::Extend)
                .fill(Color32::TRANSPARENT),
        )
    }

    /// 设置页的一行（Win11 设置应用风）：Fluent 图标 + 标题 + 说明（左），控件（右）。
    fn setting_row<F: FnOnce(&mut egui::Ui)>(
        ui: &mut egui::Ui,
        glyph: &str,
        title: &str,
        subtitle: &str,
        control: F,
    ) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            if let Some(font) = theme::icon_font() {
                ui.label(egui::RichText::new(glyph).font(font).color(theme::jade()));
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(egui::RichText::new(title).size(14.0).strong());
                ui.label(
                    egui::RichText::new(subtitle.to_string())
                        .size(12.0)
                        .color(theme::muted()),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                control(ui);
            });
        });
    }

    /// 设置页：状态卡 + 页标题 + 当前页内容 + 底部指路。
    /// 菜单在左侧（sidebar_menu_ui），这一页渲染到主区，宽屏随便排。
    pub(super) fn settings_page_ui(&mut self, ui: &mut egui::Ui) {
        // 状态卡：Win11 白卡风，不抢眼。
        egui::Frame::new()
            .fill(theme::surface())
            .stroke(egui::Stroke::new(1.0, theme::border()))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if self.running {
                        theme::status_pill(ui, theme::jade(), "运行中");
                    } else {
                        theme::status_pill(ui, theme::muted(), "已暂停");
                    }
                    let mode = if self.manual_mode() {
                        let hotkey = Hotkey::parse(&self.config.ocr_hotkey)
                            .map(|hotkey| hotkey.display())
                            .unwrap_or_else(|_| self.config.ocr_hotkey.clone());
                        format!("手动 · {hotkey}")
                    } else if self.config.realtime {
                        "自动 · 实时".to_string()
                    } else {
                        format!("自动 · 每 {} 秒", self.config.interval_seconds)
                    };
                    ui.label(
                        egui::RichText::new(mode)
                            .size(14.0)
                            .color(theme::text())
                            .strong(),
                    );
                    if let Some(seconds) = self.last_ocr_secs {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!("{seconds:.2} 秒"))
                                    .small()
                                    .color(theme::muted()),
                            );
                        });
                    }
                });
            });
        // 页标题：Win11 设置风，20 号粗体 + 图标。
        ui.horizontal(|ui| {
            let glyph = match self.sidebar_tab {
                "任务地图" => "\u{E816}",
                "游戏窗口" => "\u{E7B8}",
                "识别模式" => "\u{E721}",
                "任务类型" => "\u{E716}",
                "显示与布局" => "\u{E740}",
                "打坐计时" => "\u{E823}",
                "识别方式" => "\u{E174}",
                "方案管理" => "\u{E8F1}",
                "日志" => "\u{E8B4}",
                _ => "\u{E713}",
            };
            if let Some(font) = theme::icon_font() {
                ui.label(egui::RichText::new(glyph).font(font).color(theme::jade()));
            }
            ui.label(
                egui::RichText::new(self.sidebar_tab)
                    .size(20.0)
                    .color(theme::text())
                    .strong(),
            );
        });
        theme::divider(ui);
        // 设置页内容：一页一张白卡（Win11 控制面板分类卡）。
        // 设置页内容：Win11 设置风，一页一张白卡，卡片里是「图标+标题+说明+控件」的行。
        if self.sidebar_tab == "游戏窗口" {
            theme::card_with(theme::surface()).show(ui, |ui| {
                let current = self
                    .windows
                    .iter()
                    .find(|window| Some(window.handle) == self.selected_handle)
                    .map(|window| window.title.clone())
                    .unwrap_or_else(|| "未找到游戏窗口".into());
                let mut selected = self.selected_handle;
                Self::setting_row(
                    ui,
                    "\u{E7B8}",
                    "游戏窗口",
                    "选择要识别的游戏窗口",
                    |ui| {
                        // ComboBox::width() 只是「最小宽度」：egui 用父 Ui 的 available_width
                        // 作为文本换行宽度，默认 Extend 模式下选中项文本多长它就有多宽。
                        // 游戏窗口标题很长时，它会撑破卡片右缘并把「刷新」按钮挤出可视区，
                        // 所以这里用一个限宽的子 Ui 把可用宽度钉死，并让选中项文本截断。
                        let combo_width = (ui.available_width() - 66.0).max(100.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(combo_width, ui.spacing().interact_size.y),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                egui::ComboBox::from_id_salt("window")
                                    .selected_text(current)
                                    .width(combo_width)
                                    .truncate()
                                    .show_ui(ui, |ui| {
                                        for window in &self.windows {
                                            ui.selectable_value(
                                                &mut selected,
                                                Some(window.handle),
                                                &window.title,
                                            );
                                        }
                                    });
                            },
                        );
                        if ui.button("刷新").clicked() {
                            self.refresh_windows(true);
                            selected = self.selected_handle;
                        }
                    },
                );
                if selected != self.selected_handle {
                    self.set_window(selected);
                    self.log_detail("已切换游戏窗口，清除旧任务结果");
                }
                ui.separator();
                Self::setting_row(
                    ui,
                    "\u{E7D8}",
                    "识别区域",
                    "调整截图区域并配置图色门控",
                    |ui| {
                        if ui
                            .add_sized(
                                [ui.available_width(), 38.0],
                                egui::Button::new(
                                    egui::RichText::new(if self.sidebar_tab == "识别区域" {
                                        "识别区域已打开"
                                    } else {
                                        "设置识别区域"
                                    })
                                    .color(theme::jade())
                                    .strong(),
                                )
                                .fill(if self.sidebar_tab == "识别区域" {
                                    theme::jade_tint()
                                } else {
                                    Color32::TRANSPARENT
                                })
                                .stroke(egui::Stroke::new(1.0, theme::jade().gamma_multiply(0.55))),
                            )
                            .on_hover_text("到主区「识别区域」页调整截图区域并点击取点")
                            .clicked()
                        {
                            self.sidebar_tab = "识别区域";
                            self.capture_preview_once();
                        }
                    },
                );
                // 游戏窗口的分辨率是固定的，识别区域也固定：按分辨率记住一套区域，
                // 换分辨率时自动套用，只有需要微调时才进「设置识别区域」。
                if let Some(handle) = self.selected_handle {
                    if let Some((width, height)) = capture::client_size(handle) {
                        let key = format!("{width}x{height}");
                        let saved = self.config.region_presets.contains_key(&key);
                        ui.separator();
                        Self::setting_row(
                            ui,
                            "\u{E740}",
                            "分辨率",
                            &format!(
                                "当前 {key} · {}",
                                if saved {
                                    "已记住该分辨率的区域"
                                } else {
                                    "首次使用，调整后会自动记住"
                                }
                            ),
                            |ui| {
                                if saved && ui.button("忘记").clicked() {
                                    self.config.region_presets.remove(&key);
                                    self.config.task_region = crate::core::DEFAULT_TASK_REGION;
                                    // 记成「当前分辨率」，避免下一次识别又把默认区域存回去。
                                    self.last_client_size = Some((width, height));
                                    self.persist();
                                    self.notice = Some(format!("已恢复 {key} 的默认识别区域"));
                                }
                            },
                        );
                    }
                }
            });
            if self.windows.is_empty() {
                ui.label(
                    egui::RichText::new("请先打开游戏，窗口列表会自动刷新")
                        .small()
                        .color(theme::gold()),
                );
            }
        }
        if self.sidebar_tab == "识别模式" {
            theme::card_with(theme::surface()).show(ui, |ui| {
                let previous = self.config.trigger_mode.clone();
                Self::setting_row(
                    ui,
                    "\u{E721}",
                    "触发方式",
                    "自动按间隔触发，或手动用快捷键触发",
                    |ui| {
                        ui.selectable_value(
                            &mut self.config.trigger_mode,
                            "auto".into(),
                            "自动（按间隔）",
                        );
                        ui.selectable_value(
                            &mut self.config.trigger_mode,
                            "manual".into(),
                            "手动（快捷键）",
                        );
                    },
                );
                if previous != self.config.trigger_mode {
                    self.invalidate_scan();
                    self.sync_hotkey();
                    self.persist();
                }
                if !self.manual_mode() {
                    ui.separator();
                    Self::setting_row(
                        ui,
                        "\u{E823}",
                        "识别间隔",
                        "自动模式下每隔多少秒识别一次",
                        |ui| {
                            ui.add_enabled_ui(!self.config.realtime, |ui| {
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut self.config.interval_seconds)
                                            .range(1..=60)
                                            .suffix(" 秒"),
                                    )
                                    .on_hover_text("自动模式下每隔多少秒识别一次")
                                    .changed()
                                {
                                    self.persist();
                                }
                            });
                        },
                    );
                    ui.separator();
                    Self::setting_row(
                        ui,
                        "\u{E768}",
                        "实时模式",
                        "截图 → OCR 一完成就立刻开始下一轮（持续占用 CPU）",
                        |ui| {
                            if ui
                                .checkbox(&mut self.config.realtime, "")
                                .on_hover_text(
                                    "不等固定间隔：截图→OCR 一完成就立刻开始下一轮，                                     适合抓鬼飞行、赶路时追坐标；会持续占用 CPU / GPU",
                                )
                                .changed()
                            {
                                self.persist();
                            }
                        },
                    );
                }
                if self.manual_mode() {
                    ui.separator();
                    Self::setting_row(ui, "\u{E765}", "快捷键", "按一次触发一次识别", |ui| {
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.config.ocr_hotkey)
                                .desired_width(96.0)
                                .hint_text("F8"),
                        );
                        if response.lost_focus() {
                            self.sync_hotkey();
                            self.persist();
                        }
                        if ui
                            .button(if self.recording_hotkey { "按键…" } else { "录制" })
                            .clicked()
                        {
                            self.recording_hotkey = !self.recording_hotkey;
                        }
                    });
                    match Hotkey::parse(&self.config.ocr_hotkey) {
                        Ok(hotkey) => {
                            let (registered, warning) = match &self.hotkey_listener {
                                Some(listener) => (listener.is_alive(), listener.warning.clone()),
                                None => (false, None),
                            };
                            if registered {
                                if let Some(warn) = warning {
                                    ui.colored_label(
                                        theme::gold(),
                                        format!("⚠ {}：{}", hotkey.display(), warn),
                                    );
                                    if let Some(alt) = hotkey::suggest_available(hotkey) {
                                        if ui
                                            .button(format!("改用 {}（空闲）", alt.display()))
                                            .clicked()
                                        {
                                            self.adopt_hotkey(alt.display());
                                        }
                                    }
                                } else {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{} · 全局快捷键已注册",
                                            hotkey.display()
                                        ))
                                        .small()
                                        .color(theme::muted()),
                                    );
                                }
                            } else if !self.running {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} · 开始监听后生效",
                                        hotkey.display()
                                    ))
                                    .small()
                                    .color(theme::gold()),
                                );
                            } else if let Some(err) = self.hotkey_error.clone() {
                                ui.colored_label(theme::gold(), format!("⚠ {err}"));
                                if let Some(alt) = hotkey::suggest_available(hotkey) {
                                    if ui
                                        .button(format!("改用 {}（空闲）", alt.display()))
                                        .clicked()
                                    {
                                        self.adopt_hotkey(alt.display());
                                    }
                                }
                            } else {
                                ui.label(
                                    egui::RichText::new(format!("{} · 正在注册…", hotkey.display()))
                                        .small()
                                        .color(theme::gold()),
                                );
                            }
                        }
                        Err(err) => {
                            ui.colored_label(theme::gold(), format!("快捷键无效：{err}"));
                        }
                    }
                    ui.separator();
                    Self::setting_row(ui, "\u{E768}", "立即识别一次", "手动模式下按键触发一次 OCR", |ui| {
                        if ui.button("立即识别").clicked() {
                            self.manual_trigger();
                        }
                    });
                }
            });
        }
        if self.sidebar_tab == "任务类型" {
            // 抓鬼 / 鬼王 按任务名解析；通用任务不看任务类型，两边互斥。
            theme::card_with(theme::surface()).show(ui, |ui| {
                Self::setting_row(
                    ui,
                    "\u{E716}",
                    "抓鬼任务",
                    "识别任务栏里的抓鬼任务，并在地图上标出 NPC 坐标",
                    |ui| {
                        if ui
                            .checkbox(&mut self.config.zhuagui_enabled, "")
                            .on_hover_text("识别任务栏里的抓鬼任务，并在地图上标出 NPC 坐标")
                            .changed()
                        {
                            if self.config.zhuagui_enabled {
                                self.config.generic_enabled = false;
                            }
                            self.invalidate_scan();
                            self.persist();
                        }
                    },
                );
                ui.separator();
                Self::setting_row(
                    ui,
                    "\u{E7B4}",
                    "鬼王任务",
                    "同时识别鬼王任务，并单独显示一块地图面板",
                    |ui| {
                        if ui
                            .checkbox(&mut self.config.ghost_king_enabled, "")
                            .on_hover_text("同时识别鬼王任务，并单独显示一块地图面板")
                            .changed()
                        {
                            if self.config.ghost_king_enabled {
                                // 通用任务是独立模式，勾选鬼王时把它让开。
                                self.config.generic_enabled = false;
                            }
                            self.invalidate_scan();
                            self.persist();
                        }
                    },
                );
                ui.separator();
                Self::setting_row(
                    ui,
                    "\u{E8A5}",
                    "通用任务（只看地图名和坐标）",
                    "不识别任务类型，只按地图名和坐标标注；坐标可能落在备用区域",
                    |ui| {
                        if ui
                            .checkbox(&mut self.config.generic_enabled, "")
                            .on_hover_text(
                                "不识别任务类型，只按地图名和坐标标注；坐标不一定在第一个截图区域里，勾选后会自动启用备用区域",
                            )
                            .changed()
                        {
                            if self.config.generic_enabled {
                                // 通用任务与抓鬼/鬼王互斥；坐标可能落在第二块截图里，顺手打开备用区域。
                                self.config.zhuagui_enabled = false;
                                self.config.ghost_king_enabled = false;
                                if !self.config.extra_region_enabled {
                                    self.config.extra_region_enabled = true;
                                }
                            } else if !self.config.zhuagui_enabled && !self.config.ghost_king_enabled {
                                // 关掉通用任务后一个任务都不剩，就把抓鬼打开，免得没有面板。
                                self.config.zhuagui_enabled = true;
                            }
                            self.invalidate_scan();
                            self.persist();
                        }
                    },
                );
            });
        }
        if self.sidebar_tab == "显示与布局" {
            theme::card_with(theme::surface()).show(ui, |ui| {
                let mut high_chance_changed = false;
                if self.config.zhuagui_enabled {
                    Self::setting_row(
                        ui,
                        "\u{E816}",
                        "抓鬼：显示高概率区域",
                        "按 175LG 捉鬼省眼规则，叠加 50×50 的高概率格子",
                        |ui| {
                            high_chance_changed |= ui
                                .checkbox(&mut self.config.high_chance, "")
                                .on_hover_text(
                                    "按 175LG 捉鬼省眼规则，在抓鬼地图上叠加 50×50 的高概率格子",
                                )
                                .changed();
                        },
                    );
                }
                if self.config.ghost_king_enabled {
                    if self.config.zhuagui_enabled {
                        ui.separator();
                    }
                    Self::setting_row(
                        ui,
                        "\u{E816}",
                        "鬼王：显示高概率区域",
                        "按 175LG 捉鬼省眼规则，叠加 50×50 的高概率格子",
                        |ui| {
                            high_chance_changed |= ui
                                .checkbox(&mut self.config.ghost_king_high_chance, "")
                                .on_hover_text(
                                    "按 175LG 捉鬼省眼规则，在鬼王地图上叠加 50×50 的高概率格子",
                                )
                                .changed();
                        },
                    );
                }
                if high_chance_changed {
                    self.persist();
                }
                ui.separator();
                let previous = self.config.horizontal_layout;
                Self::setting_row(ui, "\u{E8A3}", "布局", "地图的排布方向", |ui| {
                    ui.selectable_value(&mut self.config.horizontal_layout, false, "上下");
                    ui.selectable_value(&mut self.config.horizontal_layout, true, "左右");
                });
                if previous != self.config.horizontal_layout {
                    self.persist();
                }
            });
        }
        // 浮窗参数在独立窗口「地图浮窗设置」里，设置栏底部常驻一句指路（原来在「显示与布局」页里）。
        ui.add_space(4.0);
        ui.label(
                egui::RichText::new(
                    "地图浮窗的每个任务开关、不透明度、宽度、置顶和锁定都在任务地图页下方的浮窗设置卡里（打开「地图浮窗」开关后就地出现）",
                )
                .small()
                .weak(),
            );
    }

    /// 识别方式页（主区）：标题 + 白卡（原来在「高级设置」页里的识别方式折叠区）。
    pub(super) fn mode_page_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(font) = theme::icon_font() {
                ui.label(
                    egui::RichText::new("\u{E174}")
                        .font(font)
                        .color(theme::jade()),
                );
            }
            ui.label(
                egui::RichText::new("识别方式")
                    .size(20.0)
                    .color(theme::text())
                    .strong(),
            );
        });
        theme::divider(ui);
        theme::card_with(theme::surface()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.advanced_controls(ui);
        });
    }

    /// 方案管理页（主区）：标题 + 白卡（原来在「高级设置」页里的方案管理折叠区）。
    pub(super) fn profiles_page_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(font) = theme::icon_font() {
                ui.label(
                    egui::RichText::new("\u{E8F1}")
                        .font(font)
                        .color(theme::jade()),
                );
            }
            ui.label(
                egui::RichText::new("方案管理")
                    .size(20.0)
                    .color(theme::text())
                    .strong(),
            );
        });
        theme::divider(ui);
        theme::card_with(theme::surface()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.profiles_controls(ui);
        });
    }

    pub(super) fn advanced_controls(&mut self, ui: &mut egui::Ui) {
        let previous = self.config.ocr_mode.clone();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.config.ocr_mode, "local".into(), "本地 PP-OCR");
            ui.selectable_value(&mut self.config.ocr_mode, "remote".into(), "远端 PP-OCR");
        });
        if previous != self.config.ocr_mode {
            self.invalidate_scan();
            self.notice = None;
            self.persist();
        }
        if self.config.ocr_mode == "remote" {
            ui.label(egui::RichText::new("服务地址").small().weak());
            let url_changed = ui
                .add(
                    egui::TextEdit::singleline(&mut self.api_url_edit)
                        .desired_width(f32::INFINITY)
                        .hint_text("http://192.168.1.100:8008"),
                )
                .changed();
            ui.label(egui::RichText::new("访问令牌（可选）").small().weak());
            let token_changed = ui
                .add(
                    egui::TextEdit::singleline(&mut self.api_token_edit)
                        .desired_width(f32::INFINITY)
                        .password(true)
                        .hint_text("Bearer Token"),
                )
                .changed();
            if url_changed || token_changed {
                self.config.api_url = self.api_url_edit.trim().into();
                self.config.api_token = self.api_token_edit.trim().into();
                self.remote_feedback = None;
                self.invalidate_scan();
                self.persist();
            }
            let previous = self.config.api_format.clone();
            ui.horizontal(|ui| {
                ui.label("格式");
                ui.selectable_value(&mut self.config.api_format, "multipart".into(), "multipart");
                ui.selectable_value(&mut self.config.api_format, "raw".into(), "raw");
            });
            if previous != self.config.api_format {
                self.invalidate_scan();
                self.remote_feedback = None;
                self.persist();
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.testing_remote,
                        egui::Button::new(if self.testing_remote {
                            "测试中…"
                        } else {
                            "测试连接"
                        }),
                    )
                    .clicked()
                {
                    self.test_remote_ocr();
                }
                if self.testing_remote {
                    ui.spinner();
                }
            });
            if let Some(result) = &self.remote_feedback {
                let (message, color) = match result {
                    Ok(message) => (message, theme::jade()),
                    Err(message) => (message, theme::red()),
                };
                ui.label(egui::RichText::new(message).small().color(color));
            }
        } else {
            ui.label(
                egui::RichText::new("首次识别时加载模型；缺少模型时自动下载约 25 MB。")
                    .small()
                    .weak(),
            );
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("放大倍数");
            for (value, label) in [(1, "1x"), (2, "2x"), (3, "3x")] {
                if ui
                    .selectable_value(&mut self.config.ocr_upscale, value, label)
                    .on_hover_text(
                        "送 OCR 前放大截图；只在回退到检测模型（关闭分行或分行不可信）时生效",
                    )
                    .changed()
                {
                    self.invalidate_scan();
                    self.persist();
                }
            }
        });
        if self.config.ocr_mode == "local" {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("行分割");
                for (mode, label, tip) in [
                    (
                        crate::ocr::RowMode::Auto,
                        "自动分行",
                        "投影分行 + 校验：每行单独跑识别，避开上下行粘连；分行不可信时自动回退检测模型",
                    ),
                    (
                        crate::ocr::RowMode::Manual,
                        "手动网格",
                        "按首行定位和行高切，参数在「行分割调试」页里拖",
                    ),
                    (
                        crate::ocr::RowMode::Detect,
                        "关闭",
                        "不做分行，直接走 PP-OCR「检测 + 识别」，和旧版一样",
                    ),
                ] {
                    let selected = self.config.ocr_row_mode == mode.config_value();
                    if ui
                        .selectable_label(selected, label)
                        .on_hover_text(tip)
                        .clicked()
                        && !selected
                    {
                        self.config.ocr_row_mode = mode.config_value().to_string();
                        self.invalidate_scan();
                        self.persist();
                    }
                }
            });
            if self.config.ocr_row_mode == crate::ocr::RowMode::Manual.config_value()
                && self.config.ocr_row_height < 4
            {
                ui.label(
                    egui::RichText::new(
                        "手动网格还没设行高：到「行分割调试」页拖一个行高，否则本次按自动分行",
                    )
                    .small()
                    .color(theme::gold()),
                );
            }
        }
        if self.config.ocr_mode == "local"
            && ui
                .checkbox(&mut self.config.ocr_high_recall, "高召回检测")
                .on_hover_text(
                    "降低 PP-OCR 检测阈值，减少漏字；代价是可能多出噪声框（只影响检测模型）",
                )
                .changed()
        {
            self.invalidate_scan();
            self.persist();
        }
        if ui
            .checkbox(&mut self.config.ocr_vote_frames, "多帧投票")
            .on_hover_text("地图取最近几帧众数、坐标取中位数，避免单帧错认导致跳动")
            .changed()
        {
            self.persist();
        }
        if self.config.ocr_mode == "local" {
            ui.add_space(4.0);
            match &self.last_ocr_layout {
                Some(layout) => {
                    let source = match layout.source {
                        crate::text_layout::RowSource::Projection => "投影",
                        crate::text_layout::RowSource::Cluster => "聚类兜底",
                    };
                    let color = if layout.reliable {
                        theme::jade()
                    } else {
                        theme::gold()
                    };
                    ui.label(
                        egui::RichText::new(format!(
                            "分行识别：{} 行｜行高 {}px｜{}｜{}",
                            layout.rows.len(),
                            layout.line_height,
                            source,
                            if self.config.ocr_row_mode
                                == crate::ocr::RowMode::Detect.config_value()
                            {
                                "本次关闭了分行"
                            } else if layout.reliable {
                                "走分行路径"
                            } else {
                                "回退检测模型"
                            }
                        ))
                        .small()
                        .color(color),
                    );
                }
                None => {
                    let message =
                        if self.config.ocr_row_mode == crate::ocr::RowMode::Detect.config_value() {
                            "分行识别：已关闭（直接走检测模型）"
                        } else {
                            "分行识别：还没跑过识别"
                        };
                    ui.label(egui::RichText::new(message).small().weak());
                }
            }
            if ui
                .add(theme::secondary_button("行分割调试…"))
                .on_hover_text("画出每一行的切分范围和投影曲线，确认切得对不对")
                .clicked()
            {
                self.open_rows_debug();
            }
        }
    }

    /// 日志页（主区）：标题 + 白卡：日志级别、落盘路径、实时日志（内存环形缓冲）。
    pub(super) fn log_page_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(font) = theme::icon_font() {
                ui.label(
                    egui::RichText::new("\u{E8B4}")
                        .font(font)
                        .color(theme::jade()),
                );
            }
            ui.label(
                egui::RichText::new("日志")
                    .size(20.0)
                    .color(theme::text())
                    .strong(),
            );
        });
        theme::divider(ui);
        theme::card_with(theme::surface()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label("日志级别");
                let previous = self.config.log_level.clone();
                ui.selectable_value(&mut self.config.log_level, "simple".into(), "简单");
                ui.selectable_value(&mut self.config.log_level, "detailed".into(), "详细");
                if previous != self.config.log_level {
                    self.persist();
                }
            });
            ui.label(
                egui::RichText::new(format!("日志写入：{}", self.log_path.display()))
                    .small()
                    .weak(),
            );
            ui.label(
                egui::RichText::new("简单：启动和任务启停。详细：再加 OCR 过程、报错和窗口变化。")
                    .small()
                    .weak(),
            );
            theme::divider(ui);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("实时日志").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(theme::secondary_button("清空显示"))
                        .on_hover_text("只清空界面里这份缓存，磁盘里的日志文件不受影响")
                        .clicked()
                    {
                        self.log_lines.clear();
                    }
                    if ui
                        .add(theme::secondary_button("复制日志"))
                        .on_hover_text("复制当前缓存里的日志文本")
                        .clicked()
                    {
                        ui.ctx().copy_text(
                            self.log_lines
                                .as_slices()
                                .0
                                .iter()
                                .chain(self.log_lines.as_slices().1.iter())
                                .cloned()
                                .collect::<Vec<_>>()
                                .join("\n"),
                        );
                    }
                });
            });
            if self.log_lines.is_empty() {
                ui.label(
                    egui::RichText::new("暂无日志（开始识别后，这里会实时出现日志）")
                        .small()
                        .weak(),
                );
            } else {
                let text = self
                    .log_lines
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");
                let mut text = text;
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(14)
                        .interactive(false),
                );
            }
        });
    }

    /// 方案管理页内容：识别方案选择 / 保存 / 另存为 / 删除（原「高级设置」页里的折叠区，去掉折叠层）。
    pub(super) fn profiles_controls(&mut self, ui: &mut egui::Ui) {
        self.profile_ui(ui);
        ui.label(
            egui::RichText::new("方案保存识别区域和图色门控设置。")
                .small()
                .weak(),
        );
    }

    /// 打坐计时页（主区）：标题 + 白卡，内容是 meditation_controls（原独立窗口）。
    pub(super) fn meditation_page_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(font) = theme::icon_font() {
                ui.label(
                    egui::RichText::new("\u{E823}")
                        .font(font)
                        .color(theme::jade()),
                );
            }
            ui.label(
                egui::RichText::new("打坐计时")
                    .size(20.0)
                    .color(theme::text())
                    .strong(),
            );
        });
        theme::divider(ui);
        theme::card_with(theme::surface()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.meditation_controls(ui);
        });
    }

    /// 说明页里内嵌的两张截图，缩到 900px 宽后跟程序一起编译（不依赖 assets 是否完整）。
    const HELP_IMAGE_REGION: &'static [u8] = include_bytes!("../../assets/help/set-region.png");
    const HELP_IMAGE_WINDOW: &'static [u8] = include_bytes!("../../assets/help/set-window.png");

    /// 说明页：主区滚动 + 宽度封顶 + 白卡，正文是 help_contents。
    pub(super) fn help_page_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(font) = theme::icon_font() {
                ui.label(
                    egui::RichText::new("\u{E104}")
                        .font(font)
                        .color(theme::jade()),
                );
            }
            ui.label(
                egui::RichText::new("说明")
                    .size(20.0)
                    .color(theme::text())
                    .strong(),
            );
        });
        theme::divider(ui);
        theme::card_with(theme::surface()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.help_contents(ui);
        });
    }

    /// 说明里的小标题。
    fn help_section(ui: &mut egui::Ui, title: &str) {
        if title != "基本流程" {
            ui.add_space(8.0);
        }
        ui.label(
            egui::RichText::new(title)
                .size(15.0)
                .color(theme::jade())
                .strong(),
        );
    }

    /// 内嵌截图：第一次显示时解码上传纹理，之后复用；宽度按卡片收窄，不撑破布局。
    fn help_image(
        &mut self,
        ui: &mut egui::Ui,
        key: &'static str,
        bytes: &'static [u8],
        caption: &str,
    ) {
        let texture = match self.help_textures.get(key) {
            Some(handle) => Some(handle.clone()),
            None => {
                let handle = image::load_from_memory(bytes).ok().map(|image| {
                    let rgba = image.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                    ui.ctx()
                        .load_texture(key, color, egui::TextureOptions::LINEAR)
                });
                if let Some(handle) = &handle {
                    self.help_textures.insert(key, handle.clone());
                }
                handle
            }
        };
        let Some(texture) = texture else {
            ui.label(
                egui::RichText::new("说明图片加载失败（资源打包可能不完整）")
                    .small()
                    .color(theme::gold()),
            );
            return;
        };
        let size = texture.size_vec2();
        let width = ui.available_width().min(600.0);
        let scale = (width / size.x).clamp(0.05, 1.0);
        ui.add(
            egui::Image::new(egui::load::SizedTexture::from_handle(&texture))
                .fit_to_exact_size(size * scale),
        );
        ui.label(egui::RichText::new(caption).small().weak());
        ui.add_space(6.0);
    }

    pub(super) fn help_contents(&mut self, ui: &mut egui::Ui) {
        Self::help_section(ui, "基本流程");
        for line in [
            "1. 打开游戏，程序会自动扫描并选中游戏窗口；要换窗口或重新扫描，去「页面 → 游戏窗口」（图 2）。",
            "2. 点顶栏右侧「开始识别」/「开始监听」启动，运行时同一个按钮变成「暂停识别」/「暂停监听」。",
            "3. 在「页面 → 任务类型」勾选要看的任务（抓鬼 / 鬼王 / 通用），主区就会出现对应地图面板。",
            "4. 在「页面 → 识别区域」拖动红框设置任务栏截图范围（图 1）；点「更新截图」后还能在预览里点选点位配图色门控。",
            "5. 其余都在左侧菜单栏：「显示」组管地图浮窗、窗口置顶、界面主题、精简界面；「高级设置」组管合成旗编辑器、识别方式、方案管理、行分割调试和日志。",
            "6. 想把地图单独放一边，在「显示」组打开「地图浮窗」，每个任务各开一个无边框浮窗；任务地图页下方就地出现浮窗开关、不透明度、宽度、置顶和锁定，不另开页面。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }

        Self::help_section(ui, "设置截图范围");
        ui.label(
            egui::RichText::new(
                "识别区域就是游戏任务栏所在的那块矩形。先用鼠标在预览图上拖出红框，再点「更新截图」确认框住的正是任务文字；框大了只会变慢，框小了会漏字。",
            )
            .small(),
        );
        self.help_image(
            ui,
            "help/set-region",
            Self::HELP_IMAGE_REGION,
            "图 1「页面 → 识别区域」：拖红框设置截图范围；截图会按游戏客户区尺寸记住，换分辨率后可分别保存。",
        );

        Self::help_section(ui, "设置游戏窗口");
        ui.label(
            egui::RichText::new(
                "程序按「游戏客户区」截图，所以要先选中正确的游戏窗口。多开时用下拉框选具体窗口，游戏重开后点「刷新」重新扫描。",
            )
            .small(),
        );
        self.help_image(
            ui,
            "help/set-window",
            Self::HELP_IMAGE_WINDOW,
            "图 2「页面 → 游戏窗口」：选中或刷新要识别的游戏窗口；状态栏会显示客户区尺寸，识别区域按它换算。",
        );

        Self::help_section(ui, "识别方式");
        for line in [
            "默认本地 PP-OCR（PP-OCRv5）：首次识别时加载模型；assets/models 缺失时自动下载约 25 MB。",
            "远端 PP-OCR：填服务地址和可选的 Bearer Token，点「测试连接」确认可用；支持 multipart / raw 两种请求格式。",
            "行分割默认关闭：直接走 PP-OCR「检测 + 识别」，和旧版一致。想按行切，在「高级设置 → 识别方式 → 行分割」里选「自动分行」或「手动网格」。",
            "自动分行：先投影切行，再逐行只跑识别模型，能避开上下行粘连；分行不可信时自动回退检测模型。",
            "手动网格：按「高级设置 → 行分割调试」里定的首行定位和行高切，还能在指定行后插入空白行重新对齐；那一页的「测试当前设置」会直接跑一次识别并写出逐行文本，方便对比参数。",
            "放大倍数默认 2x：只在回退到检测模型时生效；任务栏文字很小又坚持用检测模型时可以调到 3x。",
            "高召回检测：把检测阈值调低减少漏字，代价是可能多出噪声框（只影响检测模型）。",
            "多帧投票：地图取最近几帧众数、坐标取同一地图最近 3 帧的中位数，避免单帧错认导致跳动。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }

        Self::help_section(ui, "识别模式");
        for line in [
            "自动（按间隔）：按设定间隔定时识别，间隔 1~60 秒。",
            "自动 + 实时模式：勾上「实时模式」后不等间隔，截图 → OCR 一识别完就立刻开始下一轮（持续占用 CPU）。",
            "手动（快捷键）：点「开始监听」后，按快捷键触发一次识别；快捷键可填写或点「录制」。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }

        Self::help_section(ui, "界面");
        for line in [
            "界面主题：「显示」组里的「界面主题」循环切换跟随系统 / 亮色 / 暗色，默认跟随系统。",
            "精简界面：「显示」组里的「精简界面」只保留地图（浮窗开着就显示浮窗），按 Esc 恢复。",
            "地图浮窗：「显示」组里的「地图浮窗」为每个任务各开一个无边框窗口，可分别移动和等比缩放；右上角 ✕ 只关当前任务。",
            "窗口置顶：「显示」组里的「窗口置顶」让本窗口始终在其他窗口之上，方便对照游戏。",
            "说明就是本页，放在「页面」组最下面；「检查更新」在顶栏右侧，发现新版本、下载中或已下载时按钮直接显示状态。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }

        Self::help_section(ui, "图色门控");
        for line in [
            "在「页面 → 识别区域」点「更新截图」，再点击预览图取点取色。",
            "默认每 300ms 比对一次点位颜色：命中（有任务）才允许 OCR，未命中（战斗 / 任务完成）暂停 OCR。",
            "支持全部命中 / 任一命中，并可调每个点的容差与半径；未配置点位时默认允许 OCR。",
            "手动模式下手动触发会忽略图色门控。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }

        Self::help_section(ui, "其它");
        for line in [
            "任务类型：抓鬼 / 鬼王可以同时勾选；通用任务与它们互斥，单独使用。",
            "显示与布局：高概率区域开关和上下 / 左右布局；高概率红格只在没开眼（任务文字含「附近」）时才有意义。",
            "方案管理（高级设置）：保存 / 切换识别区域和图色门控，一套配置一个方案。",
            "合成旗编辑器（高级设置）：维护各大地图的飞行点，点在地图上直接加。",
            "打坐计时（页面）：启用后显示透明悬浮窗并注册快捷键。",
            "日志（高级设置）：实时看最近 1000 行日志，能清空显示、复制、看落盘路径和级别；详细级别记录完整 OCR 过程，写进 logs/rust-mhxy.log。",
            "设置改动会自动保存到程序目录的 config.json。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }
        ui.add_space(6.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("项目地址").small().weak());
            ui.hyperlink_to("GitHub", crate::APP_REPO_URL)
                .on_hover_text(crate::APP_REPO_URL);
        });
    }
}
