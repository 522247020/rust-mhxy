//! 自动更新检查、下载与安装界面。
use super::*;

/// 定时检查更新的间隔。
pub(super) const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(30 * 60);
pub(super) const UPDATE_WINDOW_SIZE: Vec2 = Vec2::new(480.0, 540.0);
/// 检查结果在按钮上停留的时间。
pub(super) const UPDATE_FLASH_DURATION: Duration = Duration::from_secs(4);

/// 自动更新的进度阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UpdatePhase {
    Idle,
    Checking,
    Available,
    Downloading,
    Downloaded,
    Failed,
}

pub(super) enum UpdateEvent {
    Checked(Result<Option<UpdateInfo>, String>),
    Downloaded(Result<PathBuf, String>),
}

impl MhxyApp {
    // ---- 自动更新 ----

    pub(super) fn start_update_check(&mut self, manual: bool) {
        if matches!(
            self.update_phase,
            UpdatePhase::Checking | UpdatePhase::Downloading
        ) {
            return;
        }
        self.last_update_check = Some(Instant::now());
        self.update_error = None;
        if manual {
            self.update_message = None;
        }
        if !self.config.update_enabled && !manual {
            return;
        }
        if self.config.update_manifest_url.trim().is_empty() {
            if manual {
                self.update_phase = UpdatePhase::Failed;
                self.update_error =
                    Some("更新地址为空，请在 config.json 设置 update/manifest_url".into());
            }
            return;
        }
        self.update_phase = UpdatePhase::Checking;
        let url = self.config.update_manifest_url.clone();
        let sender = self.update_tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let result = crate::update::check(&url, crate::APP_VERSION);
            let _ = sender.send(UpdateEvent::Checked(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn start_update_download(&mut self) {
        let Some(info) = self.update_info.clone() else {
            return;
        };
        self.update_phase = UpdatePhase::Downloading;
        self.update_error = None;
        self.update_progress.store(0, Ordering::Relaxed);
        self.update_cancel.store(false, Ordering::Relaxed);
        let destination = crate::update::temp_package(&info.version);
        let sender = self.update_tx.clone();
        let ctx = self.ctx.clone();
        let progress = self.update_progress.clone();
        let cancel = self.update_cancel.clone();
        thread::spawn(move || {
            let result = crate::update::download_to(
                &info.url,
                &destination,
                &info.sha256,
                &progress,
                &cancel,
            )
            .map(|()| destination);
            let _ = sender.send(UpdateEvent::Downloaded(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn update_dialog_ui(&mut self, ui: &mut egui::Ui) {
        let Some(info) = self.update_info.clone() else {
            ui.label("暂时没有可用的更新信息。");
            return;
        };
        ui.label(
            egui::RichText::new(format!(
                "当前 {} → 最新 {}",
                crate::APP_VERSION,
                info.version
            ))
            .strong(),
        );
        ui.label(
            egui::RichText::new(format!("安装包 {:.1} MB", info.size as f64 / 1_048_576.0))
                .small()
                .weak(),
        );
        ui.add_space(6.0);
        ui.label(egui::RichText::new("更新内容").strong());
        egui::Frame::new()
            .fill(theme::inset())
            .corner_radius(6)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("update_notes")
                    .max_height(200.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let notes = info.notes.trim();
                        if notes.is_empty() {
                            ui.label("本次更新以修复和优化为主。");
                        } else {
                            for line in notes.lines() {
                                let line = line.trim();
                                if !line.is_empty() {
                                    ui.label(format!("• {line}"));
                                }
                            }
                        }
                    });
            });
        if let Some(err) = &self.update_error {
            ui.label(egui::RichText::new(err).small().color(theme::red()));
        }
        ui.label(
            egui::RichText::new(format!("清单：{}", self.config.update_manifest_url))
                .small()
                .weak(),
        );
        ui.add_space(6.0);
        match self.update_phase {
            UpdatePhase::Downloading => {
                let total = info.size;
                let downloaded = self.update_progress.load(Ordering::Relaxed);
                let ratio = if total > 0 {
                    (downloaded as f32 / total as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                ui.add(egui::ProgressBar::new(ratio).text(format!(
                    "下载中 {:.0}% · {:.1}/{:.1} MB",
                    ratio * 100.0,
                    downloaded as f64 / 1_048_576.0,
                    total as f64 / 1_048_576.0
                )));
                if ui.button("取消下载").clicked() {
                    self.update_cancel.store(true, Ordering::Relaxed);
                }
            }
            UpdatePhase::Downloaded => {
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button("立即重启并更新")).clicked() {
                        self.install_update();
                    }
                    if ui.button("稍后").clicked() {
                        self.show_update_dialog = false;
                    }
                });
                if let Some(zip) = &self.update_zip {
                    ui.label(
                        egui::RichText::new(format!("安装包：{}", zip.display()))
                            .small()
                            .weak(),
                    );
                }
            }
            _ => {
                let retry = self.update_zip.as_ref().filter(|zip| zip.exists()).cloned();
                ui.horizontal(|ui| {
                    if let Some(_zip) = &retry {
                        if ui.add(theme::primary_button("重试安装")).clicked() {
                            self.install_update();
                        }
                    }
                    if ui.add(theme::primary_button("下载并更新")).clicked() {
                        self.start_update_download();
                    }
                    if ui.button("稍后").clicked() {
                        self.show_update_dialog = false;
                    }
                    ui.hyperlink_to(
                        egui::RichText::new("打开发布页").small(),
                        crate::update::HOME_URL,
                    );
                });
                if self.update_phase == UpdatePhase::Failed {
                    ui.label(
                        egui::RichText::new(format!(
                            "更新日志：{}",
                            crate::update::update_log_path().display()
                        ))
                        .small()
                        .weak(),
                    );
                }
            }
        }
    }

    pub(super) fn install_update(&mut self) {
        let Some(zip) = self.update_zip.clone() else {
            return;
        };
        if !zip.exists() {
            self.update_phase = UpdatePhase::Failed;
            self.update_error = Some(format!("安装包已被删除：{}", zip.display()));
            return;
        }
        self.show_update_dialog = false;
        self.flush_config();
        self.log_detail(format!("启动更新脚本，安装包：{}", zip.display()));
        match crate::update::apply_and_restart(&zip) {
            Ok(()) => {
                self.notice = Some("正在重启以完成更新…".to_string());
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                // 兜底：更新脚本在等本进程退出，窗口关闭后如果进程还赖着就把自己结束掉。
                let ctx = self.ctx.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_secs(2));
                    ctx.request_repaint();
                    std::process::exit(0);
                });
            }
            Err(err) => {
                self.update_phase = UpdatePhase::Failed;
                self.update_error = Some(format!(
                    "{err}（日志：{}）",
                    crate::update::update_log_path().display()
                ));
            }
        }
    }

    pub(super) fn poll_updates(&mut self) {
        while let Ok(event) = self.update_rx.try_recv() {
            match event {
                UpdateEvent::Checked(Ok(Some(info))) => {
                    self.update_message = Some(format!("发现新版本 {}", info.version));
                    self.update_message_at = Some(Instant::now());
                    self.log(format!(
                        "发现新版本 {}，可在右上角「检查更新」按钮更新",
                        info.version
                    ));
                    self.update_info = Some(info);
                    self.update_phase = UpdatePhase::Available;
                    self.update_error = None;
                    if self
                        .notice
                        .as_deref()
                        .is_some_and(|notice| notice.starts_with("检查更新失败"))
                    {
                        self.notice = None;
                    }
                }
                UpdateEvent::Checked(Ok(None)) => {
                    self.update_message = Some("已是最新版本".to_string());
                    self.update_message_at = Some(Instant::now());
                    self.log_detail(format!("已是最新版本（{}）", crate::APP_VERSION));
                    self.update_info = None;
                    self.update_phase = UpdatePhase::Idle;
                    self.update_error = None;
                    if self
                        .notice
                        .as_deref()
                        .is_some_and(|notice| notice.starts_with("检查更新失败"))
                    {
                        self.notice = None;
                    }
                }
                UpdateEvent::Checked(Err(err)) => {
                    self.update_phase = UpdatePhase::Failed;
                    self.update_message = Some("检查失败".to_string());
                    self.update_message_at = Some(Instant::now());
                    let url = self.config.update_manifest_url.clone();
                    let message = format!("检查更新失败：{err}（{url}）");
                    self.log_detail(message.clone());
                    self.notice = Some(message);
                    self.update_error = Some(err);
                }
                UpdateEvent::Downloaded(Ok(path)) => {
                    self.update_zip = Some(path);
                    self.update_phase = UpdatePhase::Downloaded;
                    self.update_message = Some("安装包已就绪".to_string());
                    self.update_message_at = Some(Instant::now());
                    self.update_error = None;
                }
                UpdateEvent::Downloaded(Err(err)) => {
                    self.update_phase = UpdatePhase::Failed;
                    self.update_error = Some(err);
                }
            }
        }
    }
}
