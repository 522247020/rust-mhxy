//! 键盘按键映射页面与运行状态同步。
use super::*;
use crate::config::{default_key_mappings, KeyMapping};

#[derive(Default)]
struct RowAction {
    changed: bool,
    remove: bool,
    record: Option<(usize, bool)>,
}

impl MhxyApp {
    /// 按当前配置同步全局键盘钩子。只有生效的映射变化才重建，避免每帧安装。
    pub(super) fn sync_keymap(&mut self) {
        let rules = crate::keymap::collect_rules(&self.config.key_mappings);
        let signature = crate::keymap::rules_signature(&rules);
        // 录制快捷键时临时停掉映射，否则按 F9 会被先替换成 Tab，录制到的不是用户按的键。
        let recording = self.keymap_recording.is_some()
            || self.recording_hotkey
            || self.meditation_recording_hotkey;
        let desired = if recording || rules.is_empty() {
            None
        } else {
            Some(signature)
        };
        let changed = desired != self.keymap_attempted;
        if !changed {
            if self
                .keymap_listener
                .as_ref()
                .is_some_and(|listener| listener.is_alive())
            {
                return;
            }
            if self.keymap_listener.is_some() {
                self.log_detail("按键映射钩子线程已退出，重新安装");
                self.keymap_listener = None;
                self.keymap_error = None;
            } else if desired.is_none() {
                self.keymap_error = None;
                return;
            }
        }
        let delay = if changed {
            HOTKEY_CHANGE_DELAY
        } else {
            HOTKEY_RETRY_DELAY
        };
        if self.keymap_tried_at.is_some_and(|at| at.elapsed() < delay) {
            return;
        }
        self.keymap_attempted = desired.clone();
        self.keymap_tried_at = Some(Instant::now());
        self.keymap_listener = None;
        let Some(signature) = desired else {
            self.keymap_error = None;
            return;
        };
        match crate::keymap::spawn(rules) {
            Ok(listener) => {
                self.log_detail(format!("已启用按键映射：{signature}"));
                self.keymap_listener = Some(listener);
                self.keymap_error = None;
            }
            Err(err) => {
                self.keymap_listener = None;
                // 同一个错误只记一次，避免每 5 秒刷屏。
                if self.keymap_error.as_deref() != Some(err.as_str()) {
                    self.log(format!("按键映射启用失败：{err}"));
                }
                self.keymap_error = Some(err);
            }
        }
    }

    /// 临时释放键盘钩子（录制快捷键时用），后面 sync_keymap 会按配置重新安装。
    pub(super) fn suspend_keymap(&mut self) {
        self.keymap_listener = None;
        self.keymap_attempted = None;
        self.keymap_tried_at = None;
        self.keymap_error = None;
    }

    /// 录制单条映射的源按键或目标按键；Esc 取消。
    pub(super) fn poll_keymap_recording(&mut self, ctx: &egui::Context) {
        let Some((index, source)) = self.keymap_recording else {
            return;
        };
        let Some(captured) = hotkey::capture_egui_key(ctx) else {
            return;
        };
        self.keymap_recording = None;
        if captured == "Escape" {
            return;
        }
        let Some(mapping) = self.config.key_mappings.get_mut(index) else {
            return;
        };
        if source {
            mapping.from = captured;
        } else {
            mapping.to = captured;
        }
        self.keymap_attempted = None;
        self.keymap_tried_at = None;
        self.persist();
    }

    /// 「页面 → 按键映射」：每个映射一行，带开关、源按键、目标按键和删除。
    pub(super) fn keymap_page_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(font) = theme::icon_font() {
                ui.label(
                    egui::RichText::new("\u{E765}")
                        .font(font)
                        .color(theme::jade()),
                );
            }
            ui.label(
                egui::RichText::new("按键映射")
                    .size(20.0)
                    .color(theme::text())
                    .strong(),
            );
        });
        theme::divider(ui);

        let rules = crate::keymap::collect_rules(&self.config.key_mappings);
        let enabled_count = self
            .config
            .key_mappings
            .iter()
            .filter(|mapping| mapping.enabled)
            .count();
        let listener_alive = self
            .keymap_listener
            .as_ref()
            .is_some_and(|listener| listener.is_alive());

        let mut changed = false;
        let mut remove = None;
        let mut record = None;
        let mut add = false;
        let mut reset = false;

        theme::card_with(theme::surface()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !cfg!(windows) {
                ui.label(
                    egui::RichText::new("键盘映射目前仅支持 Windows；配置会保存，换到 Windows 后生效")
                        .small()
                        .color(theme::gold()),
                );
            } else if rules.is_empty() {
                ui.label(
                    egui::RichText::new(if enabled_count == 0 {
                        "没有启用的映射".to_string()
                    } else {
                        "没有可用的映射，请检查下面的提示".to_string()
                    })
                    .small()
                    .color(theme::gold()),
                );
            } else if let Some(err) = &self.keymap_error {
                ui.colored_label(theme::gold(), format!("⚠ {err}"));
            } else if listener_alive {
                ui.label(
                    egui::RichText::new(format!(
                        "已启用 {} 条映射 · 游戏窗口在前台时同样生效",
                        rules.len()
                    ))
                    .small()
                    .color(theme::jade()),
                );
            } else {
                ui.label(
                    egui::RichText::new("正在安装键盘钩子…")
                        .small()
                        .color(theme::gold()),
                );
            }
            ui.label(
                egui::RichText::new(
                    "把源按键替换成目标按键，例如 F9 → Tab。可填 F1~F24、字母数字、Tab、空格、方向键等；只支持单键，不支持组合键。",
                )
                .small()
                .weak(),
            );
            ui.separator();

            let count = self.config.key_mappings.len();
            for index in 0..count {
                let action = {
                    let mapping = &mut self.config.key_mappings[index];
                    Self::keymap_row_ui(ui, index, mapping, self.keymap_recording)
                };
                changed |= action.changed;
                remove = action.remove.then_some(index).or(remove);
                if let Some(slot) = action.record {
                    record = Some(slot);
                }
                if index + 1 < count {
                    ui.separator();
                }
            }
            if count == 0 {
                ui.label(
                    egui::RichText::new("还没有映射，点下面的「添加映射」新增一条")
                        .small()
                        .color(theme::muted()),
                );
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                add = ui.button("添加映射").clicked();
                reset = ui.button("恢复默认映射").clicked();
            });
        });

        if let Some((index, source)) = record {
            let slot = Some((index, source));
            self.keymap_recording = if self.keymap_recording == slot {
                None
            } else {
                slot
            };
            if self.keymap_recording.is_some() {
                // 同一时间只录一个快捷键，避免和识别 / 打坐录制抢按键。
                self.recording_hotkey = false;
                self.meditation_recording_hotkey = false;
                // 先摘掉钩子，保证录到的是用户实际按下的物理键，而不是映射后的键。
                self.suspend_keymap();
            }
        }
        if let Some(index) = remove {
            self.config.key_mappings.remove(index);
            if let Some((recording_index, source)) = self.keymap_recording {
                if recording_index == index {
                    self.keymap_recording = None;
                } else if recording_index > index {
                    self.keymap_recording = Some((recording_index - 1, source));
                }
            }
            changed = true;
        }
        if reset {
            self.config.key_mappings = default_key_mappings();
            self.keymap_recording = None;
            changed = true;
        }
        if add && self.config.key_mappings.len() < 32 {
            self.config.key_mappings.push(KeyMapping::default());
            changed = true;
        }
        if changed {
            self.keymap_attempted = None;
            self.keymap_tried_at = None;
            self.persist();
        }
        if self.keymap_recording.is_some() {
            ui.label(
                egui::RichText::new("请按下要映射的按键（Esc 取消）")
                    .small()
                    .color(theme::gold()),
            );
        }
    }

    fn keymap_row_ui(
        ui: &mut egui::Ui,
        index: usize,
        mapping: &mut KeyMapping,
        recording: Option<(usize, bool)>,
    ) -> RowAction {
        let mut action = RowAction::default();
        ui.horizontal(|ui| {
            action.changed |= ui
                .checkbox(&mut mapping.enabled, "")
                .on_hover_text("关闭后这一条映射不生效")
                .changed();
            let source_label = if recording == Some((index, true)) {
                "按键…"
            } else {
                "录制"
            };
            action.changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut mapping.from)
                        .desired_width(64.0)
                        .hint_text("F9"),
                )
                .on_hover_text("源按键")
                .changed();
            if ui
                .add(egui::Button::new(source_label).small())
                .on_hover_text("按下要映射的源按键")
                .clicked()
            {
                action.record = Some((index, true));
            }
            ui.label(egui::RichText::new("→").size(14.0).color(theme::muted()));
            action.changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut mapping.to)
                        .desired_width(64.0)
                        .hint_text("Tab"),
                )
                .on_hover_text("目标按键")
                .changed();
            let target_label = if recording == Some((index, false)) {
                "按键…"
            } else {
                "录制"
            };
            if ui
                .add(egui::Button::new(target_label).small())
                .on_hover_text("按下目标按键")
                .clicked()
            {
                action.record = Some((index, false));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(theme::danger_button("删除").small())
                    .on_hover_text("删除这一条映射")
                    .clicked()
                {
                    action.remove = true;
                }
            });
        });
        if !mapping.enabled {
            ui.label(egui::RichText::new("已关闭").small().color(theme::muted()));
        } else if let Err(err) = crate::keymap::parse_rule(mapping) {
            ui.colored_label(theme::gold(), format!("⚠ {err}"));
        }
        action
    }
}
