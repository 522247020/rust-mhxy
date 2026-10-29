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

    /// 日志直接写盘，界面不再展示；详细级别才记录 OCR 过程。
    pub(super) fn push_log(&mut self, simple: bool, message: String) {
        if !simple && self.config.log_level != "detailed" {
            return;
        }
        let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        append_log(&self.log_path, &format!("[{time}] {message}\n"));
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
        self.config.preprocess_mode = profile.preprocess_mode;
        self.config.preprocess_color = profile.preprocess_color;
        self.config.preprocess_tolerance = profile.preprocess_tolerance;
        self.config.extra_preprocess_mode = profile.extra_preprocess_mode;
        self.config.extra_preprocess_color = profile.extra_preprocess_color;
        self.config.extra_preprocess_tolerance = profile.extra_preprocess_tolerance;
        self.preprocess_color_edit = gate::to_hex(self.config.preprocess_color);
        self.extra_preprocess_color_edit = gate::to_hex(self.config.extra_preprocess_color);
        self.invalidate_scan();
        self.check_gate();
        self.refresh_preprocess_preview();
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
                .on_hover_text("把当前的图色门控和预处理设置保存到当前方案")
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
                    egui::Button::new("删除").small(),
                )
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
    pub(super) fn controls(&mut self, ui: &mut egui::Ui) {
        // 开始 / 暂停固定在顶栏，侧栏不再重复主按钮，只保留常用设置。
        egui::ScrollArea::vertical()
            .id_salt("sidebar")
            .auto_shrink([false, false])
            .show(ui, |ui| self.basic_controls(ui));
    }

    pub(super) fn basic_controls(&mut self, ui: &mut egui::Ui) {
        let fold_window = self.sidebar_section_open("window");
        let now =
            theme::collapsible_section(ui, "sidebar_window", "游戏窗口", fold_window, |ui| {
                let current = self
                    .windows
                    .iter()
                    .find(|window| Some(window.handle) == self.selected_handle)
                    .map(|window| window.title.clone())
                    .unwrap_or_else(|| "未找到游戏窗口".into());
                let mut selected = self.selected_handle;
                ui.horizontal(|ui| {
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
                });
                if selected != self.selected_handle {
                    self.set_window(selected);
                    self.log_detail("已切换游戏窗口，清除旧任务结果");
                }
                if self.windows.is_empty() {
                    ui.label(
                        egui::RichText::new("请先打开游戏，窗口列表会自动刷新")
                            .small()
                            .color(theme::gold()),
                    );
                }
                let label = if self.show_region_window {
                    "关闭识别区域"
                } else {
                    "设置识别区域"
                };
                if ui
                    .add_sized([ui.available_width(), 32.0], egui::Button::new(label))
                    .clicked()
                {
                    self.toggle_region_window(ui.ctx());
                }
                // 游戏窗口的分辨率是固定的，识别区域也固定：按分辨率记住一套区域，
                // 换分辨率时自动套用，只有需要微调时才进「设置识别区域」。
                if let Some(handle) = self.selected_handle {
                    if let Some((width, height)) = capture::client_size(handle) {
                        let key = format!("{width}x{height}");
                        let saved = self.config.region_presets.contains_key(&key);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new(format!("当前分辨率 {key}"))
                                    .small()
                                    .color(theme::muted()),
                            );
                            ui.label(
                                egui::RichText::new(if saved {
                                    "已记住该分辨率的区域"
                                } else {
                                    "首次使用，调整后会自动记住"
                                })
                                .small()
                                .color(if saved {
                                    theme::jade()
                                } else {
                                    theme::muted()
                                }),
                            );
                            if saved && ui.small_button("忘记").clicked() {
                                self.config.region_presets.remove(&key);
                                self.config.task_region = crate::core::DEFAULT_TASK_REGION;
                                // 记成「当前分辨率」，避免下一次识别又把默认区域存回去。
                                self.last_client_size = Some((width, height));
                                self.persist();
                                self.notice = Some(format!("已恢复 {key} 的默认识别区域"));
                            }
                        });
                    }
                }
            });
        if now != fold_window {
            self.set_sidebar_section_open("window", now);
        }
        let fold_trigger = self.sidebar_section_open("trigger");
        let now =
            theme::collapsible_section(ui, "sidebar_trigger", "识别模式", fold_trigger, |ui| {
                let previous = self.config.trigger_mode.clone();
                ui.horizontal(|ui| {
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
                });
                if previous != self.config.trigger_mode {
                    self.invalidate_scan();
                    self.sync_hotkey();
                    self.persist();
                }
                if !self.manual_mode() {
                    ui.horizontal(|ui| {
                        ui.label("识别间隔");
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
                }
                if self.manual_mode() {
                    ui.horizontal(|ui| {
                        ui.label("快捷键");
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.config.ocr_hotkey)
                                .desired_width(96.0)
                                .hint_text("F8"),
                        );
                        if response.lost_focus() {
                            self.sync_hotkey();
                            self.persist();
                        }
                        let record_label = if self.recording_hotkey {
                            "按键…"
                        } else {
                            "录制"
                        };
                        if ui.button(record_label).clicked() {
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
                                    egui::RichText::new(format!(
                                        "{} · 正在注册…",
                                        hotkey.display()
                                    ))
                                    .small()
                                    .color(theme::gold()),
                                );
                            }
                        }
                        Err(err) => {
                            ui.colored_label(theme::gold(), format!("快捷键无效：{err}"));
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui.button("立即识别一次").clicked() {
                            self.manual_trigger();
                        }
                        ui.label(
                            egui::RichText::new("手动模式下按键触发一次 OCR")
                                .small()
                                .weak(),
                        );
                    });
                }
            });
        if now != fold_trigger {
            self.set_sidebar_section_open("trigger", now);
        }
        let fold_task = self.sidebar_section_open("task");
        let now = theme::collapsible_section(ui, "sidebar_task", "任务类型", fold_task, |ui| {
            // 抓鬼 / 鬼王 按任务名解析；通用任务不看任务类型，两边互斥。
            if ui
                .checkbox(&mut self.config.zhuagui_enabled, "抓鬼任务")
                .on_hover_text("识别任务栏里的抓鬼任务，并在地图上标出 NPC 坐标")
                .changed()
            {
                if self.config.zhuagui_enabled {
                    self.config.generic_enabled = false;
                }
                self.invalidate_scan();
                self.persist();
            }
            if ui
                .checkbox(&mut self.config.ghost_king_enabled, "鬼王任务")
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
            if ui
                        .checkbox(&mut self.config.generic_enabled, "通用任务（只看地图名和坐标）")
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
        });
        if now != fold_task {
            self.set_sidebar_section_open("task", now);
        }
        let fold_display = self.sidebar_section_open("display");
        let now = theme::collapsible_section(
            ui,
            "sidebar_display",
            "显示与布局",
            fold_display,
            |ui| {
                let mut high_chance_changed = false;
                if self.config.zhuagui_enabled {
                    high_chance_changed |= ui
                        .checkbox(&mut self.config.high_chance, "抓鬼：显示高概率区域")
                        .on_hover_text("按 175LG 捉鬼省眼规则，在抓鬼地图上叠加 50×50 的高概率格子")
                        .changed();
                }
                if self.config.ghost_king_enabled {
                    high_chance_changed |= ui
                        .checkbox(
                            &mut self.config.ghost_king_high_chance,
                            "鬼王：显示高概率区域",
                        )
                        .on_hover_text("按 175LG 捉鬼省眼规则，在鬼王地图上叠加 50×50 的高概率格子")
                        .changed();
                }
                if high_chance_changed {
                    self.persist();
                }
                let previous = self.config.horizontal_layout;
                ui.horizontal(|ui| {
                    ui.label("布局");
                    ui.selectable_value(&mut self.config.horizontal_layout, false, "上下");
                    ui.selectable_value(&mut self.config.horizontal_layout, true, "左右");
                });
                if previous != self.config.horizontal_layout {
                    self.persist();
                }
                // 浮窗参数已经并入顶栏「更多 → 显示」，这里只留一句指路，不重复控件。
                ui.label(
                    egui::RichText::new(
                        "地图浮窗的开关、不透明度、置顶和锁定都在顶栏「更多 → 显示」里",
                    )
                    .small()
                    .weak(),
                );
            },
        );
        if now != fold_display {
            self.set_sidebar_section_open("display", now);
        }
        ui.add_space(4.0);
    }

    /// 高级设置窗口：识别方式、方案管理、合成旗，以及日志级别。
    pub(super) fn advanced_controls(&mut self, ui: &mut egui::Ui) {
        let mode = if self.config.ocr_mode == "remote" {
            "远端 PP-OCR"
        } else {
            "本地 PP-OCRv5"
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(format!("当前识别：{mode}"))
                    .small()
                    .color(theme::muted()),
            );
            ui.label(
                egui::RichText::new("· 识别方式、方案与合成旗")
                    .small()
                    .weak(),
            );
        });
        ui.add_space(2.0);
        let fold_ocr = self.sidebar_section_open("ocr");
        let now = theme::collapsible_section(ui, "sidebar_ocr", "识别方式", fold_ocr, |ui| {
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
                    ui.selectable_value(
                        &mut self.config.api_format,
                        "multipart".into(),
                        "multipart",
                    );
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
                        .on_hover_text("送 OCR 前放大截图；任务栏文字小时提高检测召回")
                        .changed()
                    {
                        self.invalidate_scan();
                        self.persist();
                    }
                }
            });
            if self.config.ocr_mode == "local"
                && ui
                    .checkbox(&mut self.config.ocr_high_recall, "高召回检测")
                    .on_hover_text("降低 PP-OCR 检测阈值，减少漏字；代价是可能多出噪声框")
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
        });
        if now != fold_ocr {
            self.set_sidebar_section_open("ocr", now);
        }
        let fold_profiles = self.sidebar_section_open("profiles");
        let now = theme::collapsible_section(
            ui,
            "sidebar_profiles",
            "方案管理",
            fold_profiles,
            |ui| {
                self.profile_ui(ui);
                ui.label(
                    egui::RichText::new("方案保存识别区域、图色门控和预处理设置。")
                        .small()
                        .weak(),
                );
            },
        );
        if now != fold_profiles {
            self.set_sidebar_section_open("profiles", now);
        }
        let fold_flight = self.sidebar_section_open("flight_flags");
        let now = theme::collapsible_section(
            ui,
            "sidebar_flight_flags",
            "合成旗标注",
            fold_flight,
            |ui| self.flight_flags_ui(ui),
        );
        if now != fold_flight {
            self.set_sidebar_section_open("flight_flags", now);
        }
        ui.separator();
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
    }

    pub(super) fn help_contents(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("基本流程").strong());
        for line in [
            "1. 打开游戏，程序会自动扫描并选中游戏窗口（也可用下拉框选择、点「刷新」）。",
            "2. 点顶栏右侧「开始识别」/「开始监听」启动，运行时同一个按钮变成「暂停识别」/「暂停监听」。",
            "3. 在「任务类型」里勾选要显示的任务，侧栏会自动出现对应地图面板。",
            "4. 基础区「设置识别区域」打开独立窗口，可调整任务栏截图区域并点击取点配置图色门控。",
            "5. 识别方式、方案和日志在顶栏「更多 → 设置 → 高级设置」里；打坐计时和合成旗编辑器在「更多 → 工具」里。",
            "6. 想把地图单独放一边，在顶栏「更多 → 显示」里选「地图浮窗」，每个任务各开一个无边框浮窗；不透明度、浮窗置顶和锁定就在同一分组里调。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new("识别模式").strong());
        for line in [
            "自动（按间隔）：按设定间隔定时识别。",
            "手动（快捷键）：点「开始监听」后，按快捷键触发一次识别；快捷键可填写或点「录制」。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new("界面").strong());
        for line in [
            "界面主题：顶栏「更多 → 显示 → 界面主题」可选跟随系统 / 亮色 / 暗色，默认跟随系统，系统切换亮暗时立即跟随。",
            "精简界面：顶栏「更多 → 显示 → 精简界面」隐藏按钮栏、状态栏和设置栏，只保留地图；按 Esc 恢复默认界面。",
            "地图浮窗：顶栏「更多 → 显示 → 地图浮窗」为每个任务各开一个无边框窗口，可分别移动和等比缩放；Windows 用系统原生缩放，宽度可输入或选预设。右上角 ✕ 只关当前任务；透明度、浮窗置顶和锁定在同一分组里。",
            "窗口置顶：顶栏「更多 → 显示 → 窗口置顶」让本窗口始终保持在其他窗口之上，方便对照游戏。",
            "帮助：顶栏「更多 → 帮助」里有使用说明和更新；发现新版本、下载中或已下载时，「更多」按钮会直接显示更新状态。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new("图色门控").strong());
        for line in [
            "在「设置识别区域」窗口里点「手动截图一次」，再点击预览图取点取色。",
            "默认每 300ms 比对一次点位颜色：命中（有任务）才允许 OCR，未命中（战斗 / 任务完成）暂停 OCR。",
            "支持全部命中 / 任一命中，并可调每个点的容差与半径；未配置点位时默认允许 OCR。",
            "手动模式下手动触发会忽略图色门控。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new("其它").strong());
        for line in [
            "任务类型：抓鬼 / 鬼王可以同时勾选；通用任务与它们互斥，单独使用。",
            "显示与布局（基础区）：高概率区域开关和上下 / 左右布局；地图浮窗的开关和浮窗参数不在这里重复，统一在顶栏「更多 → 显示」里。",
            "设置（顶栏「更多 → 设置 → 高级设置」）：识别方式、方案管理、合成旗标注和日志级别。",
            "工具（顶栏「更多 → 工具」）：打坐计时和合成旗编辑器；打坐计时启用后显示透明悬浮窗并注册快捷键。",
            "帮助（顶栏「更多 → 帮助」）：使用说明和更新；更新发现新版本、下载中或已下载时，「更多」按钮会直接显示状态。",
            "日志：直接写入配置目录下的 logs/rust-mhxy.log，不在界面显示；详细级别记录 OCR 过程。",
            "设置会自动保存到程序目录的 config.json。",
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
