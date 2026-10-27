//! 取点窗口、客户区预览与预处理界面。
use super::*;

impl MhxyApp {
    pub(super) fn toggle_region_window(&mut self, ctx: &egui::Context) {
        if self.show_region_window {
            self.show_region_window = false;
            self.region_window_pos = None;
            self.applied_region_on_top = None;
            self.release_preview();
        } else {
            self.show_region_window = true;
            // 取点窗口优先放在主窗口右侧；右侧不够就放左侧，
            // 两边都放不下才贴屏幕右缘（此时靠置顶层级保证不被主窗口压住）。
            self.region_window_pos = Some(region_window_position(ctx, REGION_WINDOW_SIZE));
            self.applied_region_on_top = None;
            self.capture_preview_once();
        }
    }

    /// 画客户区预览：红框主区域、蓝框备用区域、图色点位；点击时返回 (x, y, 颜色)。
    pub(super) fn preview(
        &self,
        ui: &mut egui::Ui,
        max_height: f32,
    ) -> Option<(i32, i32, [u8; 3])> {
        let Some(image) = self.preview_image.as_ref() else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("暂无截图\n选择游戏窗口后，点击「更新截图」")
                        .color(theme::muted()),
                );
            });
            return None;
        };
        let texture = self.preview_texture.as_ref()?;
        let size = Vec2::new(image.width() as f32, image.height() as f32);
        let available = vec2(
            ui.available_width(),
            ui.available_height().min(max_height).max(1.0),
        );
        let scale = (available.x / size.x).min(available.y / size.y).max(0.001);
        let (rect, response) = ui.allocate_exact_size(size * scale, Sense::click());
        let painter = ui.painter_at(rect);
        let target = fit_rect(rect, size);
        painter.image(
            texture.id(),
            target,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let draw_scale = target.width() / size.x;
        let to_screen =
            |x: f32, y: f32| pos2(target.min.x + x * draw_scale, target.min.y + y * draw_scale);
        for (region, color, enabled) in [
            (self.config.task_region, theme::red(), true),
            (
                self.config.extra_task_region,
                Color32::from_rgb(100, 174, 255),
                self.config.extra_region_enabled,
            ),
        ] {
            if !enabled {
                continue;
            }
            let (left, top, right, bottom) = crate::core::task_region(size.x, size.y, region);
            painter.rect_stroke(
                Rect::from_min_max(to_screen(left, top), to_screen(right, bottom)),
                CornerRadius::ZERO,
                egui::Stroke::new(2.0, color),
                egui::StrokeKind::Inside,
            );
        }

        // 鼠标悬停：显示客户区坐标与该像素颜色；点击则可取点。
        let mut picked = None;
        if let Some(pos) = response.hover_pos().filter(|pos| target.contains(*pos)) {
            let px = ((pos.x - target.min.x) / draw_scale).floor() as i32;
            let py = ((pos.y - target.min.y) / draw_scale).floor() as i32;
            if let Some(color) = sample_image(image, px, py) {
                painter.circle_stroke(
                    to_screen(px as f32, py as f32),
                    5.0,
                    egui::Stroke::new(1.5, Color32::WHITE),
                );
                painter.text(
                    target.left_top() + vec2(8.0, 8.0),
                    Align2::LEFT_TOP,
                    format!("({px}, {py})  {}", gate::to_hex(color)),
                    FontId::proportional(13.0),
                    Color32::WHITE,
                );
                if (self.pick_points || self.pick_text_color) && response.clicked() {
                    picked = Some((px, py, color));
                }
            }
        }

        // 已配置的点位，标在地图上。
        for point in &self.config.color_gate_points {
            let center = to_screen(point.x as f32, point.y as f32);
            let box_rect = Rect::from_center_size(center, vec2(9.0, 9.0));
            let color = Color32::from_rgb(point.color[0], point.color[1], point.color[2]);
            painter.rect_filled(box_rect, CornerRadius::ZERO, color);
            painter.rect_stroke(
                box_rect,
                CornerRadius::ZERO,
                egui::Stroke::new(1.5, Color32::WHITE),
                egui::StrokeKind::Outside,
            );
        }
        picked
    }

    /// 识别区域窗口：
    /// 上排「截图范围 / 图色门控 | 识别区域预览」，分隔线下面是「识别预处理 | 预处理预览」。
    pub(super) fn region_window_ui(&mut self, ui: &mut egui::Ui) {
        if ui.ctx().input(|input| input.viewport().close_requested()) {
            self.show_region_window = false;
            self.region_window_pos = None;
            self.release_preview();
        }
        egui::Frame::central_panel(ui.style())
            .inner_margin(16)
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.horizontal_wrapped(|ui| {
                    ui.heading("识别区域");
                    if ui.button("更新截图").clicked() {
                        self.capture_preview_once();
                    }
                    if ui
                        .add_enabled(
                            !self.ocr_test_running,
                            egui::Button::new(if self.ocr_test_running {
                                "识别中…"
                            } else {
                                "测试 OCR"
                            }),
                        )
                        .on_hover_text("用当前截图和预处理设置识别一次，不影响正式识别")
                        .clicked()
                    {
                        self.run_panel_test();
                    }
                    ui.checkbox(&mut self.pick_points, "点击预览取点");
                    if self.pick_text_color {
                        ui.label(
                            egui::RichText::new("取色模式：点击右侧预览设置文字颜色")
                                .small()
                                .color(theme::jade()),
                        );
                    }
                });
                ui.separator();
                self.profile_ui(ui);
                if let Some(message) = &self.notice {
                    ui.label(egui::RichText::new(message).small().color(theme::gold()));
                }
                if self.ocr_test_running {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("正在识别…").small().weak());
                    });
                } else if let Some(result) = self.ocr_test_result.clone() {
                    let mut clear = false;
                    egui::Frame::new()
                        .fill(theme::inset())
                        .corner_radius(6)
                        .inner_margin(8)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("测试结果").small().strong());
                                if ui.small_button("清空").clicked() {
                                    clear = true;
                                }
                            });
                            egui::ScrollArea::vertical()
                                .id_salt("test_result")
                                .max_height(80.0)
                                .show(ui, |ui| match &result {
                                    Ok(text) => {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(text)
                                                    .monospace()
                                                    .size(11.0)
                                                    .color(theme::text()),
                                            )
                                            .wrap(),
                                        );
                                    }
                                    Err(err) => {
                                        ui.label(
                                            egui::RichText::new(err).small().color(theme::red()),
                                        );
                                    }
                                });
                        });
                    if clear {
                        self.ocr_test_result = None;
                    }
                }

                let total_height = ui.available_height();
                let separator = 16.0;
                let top_height = ((total_height - separator) * 0.62).max(240.0);
                let bottom_height = (total_height - separator - top_height).max(140.0);
                let settings_width = (ui.available_width() * 0.36).clamp(300.0, 430.0);

                // 上排：截图范围 / 图色门控 | 识别区域预览
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), top_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal_top(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width(settings_width);
                                egui::ScrollArea::vertical()
                                    .id_salt("region_top_settings")
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        let mut fold_changed = false;
                                        let scope_open = theme::collapsible_section(
                                            ui,
                                            "region_scope",
                                            "截图范围",
                                            self.config.region_scope_open,
                                            |ui| {
                                                let mut changed = region_fields(
                                                    ui,
                                                    "main",
                                                    "主区域",
                                                    &mut self.config.task_region,
                                                );
                                                ui.add_space(6.0);
                                                ui.label(egui::RichText::new("备用区域").strong());
                                                changed |= ui
                                                    .checkbox(
                                                        &mut self.config.extra_region_enabled,
                                                        "启用备用区域",
                                                    )
                                                    .changed();
                                                ui.add_enabled_ui(
                                                    self.config.extra_region_enabled,
                                                    |ui| {
                                                        changed |= region_grid(
                                                            ui,
                                                            "extra",
                                                            &mut self.config.extra_task_region,
                                                        );
                                                    },
                                                );
                                                if changed {
                                                    self.invalidate_scan();
                                                    self.persist();
                                                    self.refresh_preprocess_preview();
                                                }
                                            },
                                        );
                                        if scope_open != self.config.region_scope_open {
                                            self.config.region_scope_open = scope_open;
                                            fold_changed = true;
                                        }
                                        let gate_open = theme::collapsible_section(
                                            ui,
                                            "region_gate",
                                            "图色门控设置",
                                            self.config.region_gate_open,
                                            |ui| self.color_gate_ui(ui),
                                        );
                                        if gate_open != self.config.region_gate_open {
                                            self.config.region_gate_open = gate_open;
                                            fold_changed = true;
                                        }
                                        if fold_changed {
                                            self.persist();
                                        }
                                    });
                            });
                            ui.separator();
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new("识别区域预览").strong());
                                ui.label(
                                    egui::RichText::new(
                                        "红框=主区域，蓝框=备用区域；点击图片可取点",
                                    )
                                    .small()
                                    .weak(),
                                );
                                let preview_height = (ui.available_height() - 8.0).max(120.0);
                                if let Some((x, y, color)) = self.preview(ui, preview_height) {
                                    self.apply_preview_click(x, y, color);
                                }
                            });
                        });
                    },
                );
                ui.separator();
                // 下排：识别预处理 | 预处理预览
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), bottom_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal_top(|ui| {
                            let total = ui.available_width();
                            let preprocess_width = (total * 0.38).clamp(260.0, 420.0);
                            // 识别预处理
                            ui.vertical(|ui| {
                                ui.set_width(preprocess_width);
                                egui::ScrollArea::vertical()
                                    .id_salt("region_preprocess_settings")
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        let open = theme::collapsible_section(
                                            ui,
                                            "region_preprocess",
                                            "识别预处理",
                                            self.config.region_preprocess_open,
                                            |ui| self.preprocess_ui(ui),
                                        );
                                        if open != self.config.region_preprocess_open {
                                            self.config.region_preprocess_open = open;
                                            self.persist();
                                        }
                                    });
                            });
                            ui.separator();
                            // 预处理预览（主区域 / 备用区域左右排列）
                            ui.vertical(|ui| {
                                self.preprocess_preview_ui(
                                    ui,
                                    (ui.available_height() - 8.0).max(80.0),
                                );
                            });
                        });
                    },
                );
            });
    }

    pub(super) fn apply_preview_click(&mut self, x: i32, y: i32, color: [u8; 3]) {
        if self.pick_text_color {
            let extra = self.preprocess_target_extra && self.config.extra_region_enabled;
            if extra {
                self.config.extra_preprocess_color = color;
            } else {
                self.config.preprocess_color = color;
            }
            self.preprocess_color_edit = gate::to_hex(color);
            self.pick_text_color = false;
            let mut pipeline = self.preprocess_pipeline_for(extra);
            if !pipeline.color_key {
                pipeline.color_key = true;
                if extra {
                    self.config.extra_preprocess_mode = pipeline.config_value();
                } else {
                    self.config.preprocess_mode = pipeline.config_value();
                }
            }
            self.invalidate_scan();
            self.persist();
            self.refresh_preprocess_preview();
            self.log_detail(format!(
                "取文字颜色 {}，预处理：{}",
                gate::to_hex(color),
                pipeline.summary()
            ));
        } else {
            let mut point = ColorPoint::new(x, y, color, self.config.color_gate_tolerance, 1);
            point.label = format!("点{}", self.config.color_gate_points.len() + 1);
            self.config.color_gate_points.push(point);
            self.check_gate();
            self.persist();
            self.log_detail(format!("新增图色点位 ({x}, {y}) {}", gate::to_hex(color)));
        }
    }

    pub(super) fn preprocess_preview_ui(&self, ui: &mut egui::Ui, max_height: f32) {
        ui.label(egui::RichText::new("预处理预览").small().weak());
        let width = ui.available_width();
        let column = ((width - 16.0) / 2.0).max(80.0);
        let image_height = (max_height - 22.0).max(60.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(column);
                ui.label(egui::RichText::new("主区域").small().color(theme::muted()));
                preview_texture(
                    ui,
                    &self.preprocess_preview,
                    column,
                    image_height,
                    "点击「更新截图」后可预览",
                );
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(column);
                ui.label(
                    egui::RichText::new("备用区域")
                        .small()
                        .color(theme::muted()),
                );
                if self.config.extra_region_enabled {
                    preview_texture(
                        ui,
                        &self.preprocess_preview_extra,
                        column,
                        image_height,
                        "暂无备用区域截图",
                    );
                } else {
                    ui.label(egui::RichText::new("未启用备用区域").small().weak());
                }
            });
        });
    }

    pub(super) fn preprocess_pipeline_for(&self, extra: bool) -> preprocess::Pipeline {
        let mode = if extra {
            &self.config.extra_preprocess_mode
        } else {
            &self.config.preprocess_mode
        };
        preprocess::Pipeline::from_config(mode)
    }

    pub(super) fn preprocess_ui(&mut self, ui: &mut egui::Ui) {
        self.preprocess_block(ui, false, true);
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        self.preprocess_block(ui, true, self.config.extra_region_enabled);
    }

    /// 单个区域的预处理设置。`extra` 未启用时整块灰色不可选。
    pub(super) fn preprocess_block(&mut self, ui: &mut egui::Ui, extra: bool, enabled: bool) {
        let title = if extra {
            "备用区域预处理"
        } else {
            "主区域预处理"
        };
        ui.add_enabled_ui(enabled, |ui| {
            ui.label(egui::RichText::new(title).strong().color(if enabled {
                theme::text()
            } else {
                theme::muted()
            }));
            if !enabled {
                ui.label(
                    egui::RichText::new("先在截图范围勾选「启用备用区域」")
                        .small()
                        .weak(),
                );
                return;
            }
            let previous_mode = if extra {
                self.config.extra_preprocess_mode.clone()
            } else {
                self.config.preprocess_mode.clone()
            };
            let mut pipeline = preprocess::Pipeline::from_config(&previous_mode);
            let mut color = if extra {
                self.config.extra_preprocess_color
            } else {
                self.config.preprocess_color
            };
            let mut tolerance = if extra {
                self.config.extra_preprocess_tolerance
            } else {
                self.config.preprocess_tolerance
            };
            let mut edit_value = if extra {
                self.extra_preprocess_color_edit.clone()
            } else {
                self.preprocess_color_edit.clone()
            };
            let mut changed = false;
            ui.horizontal_wrapped(|ui| {
                changed |= ui.checkbox(&mut pipeline.color_key, "颜色提取").changed();
                changed |= ui.checkbox(&mut pipeline.grayscale, "灰度").changed();
                changed |= ui.checkbox(&mut pipeline.binarize, "二值化").changed();
                changed |= ui
                    .checkbox(&mut pipeline.invert, "反色")
                    .on_hover_text("最终黑白颠倒：二值化把文字判成白色时勾选")
                    .changed();
            });
            if pipeline.color_key {
                let mut new_color: Option<[u8; 3]> = None;
                ui.horizontal_wrapped(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(3),
                        Color32::from_rgb(color[0], color[1], color[2]),
                    );
                    ui.painter().rect_stroke(
                        rect,
                        CornerRadius::same(3),
                        egui::Stroke::new(1.0, theme::border()),
                        egui::StrokeKind::Outside,
                    );
                    if ui
                        .selectable_label(self.pick_text_color, "点击预览取色")
                        .on_hover_text("在右侧预览图上点任务文字取色；点不准时用色值或调色板")
                        .clicked()
                    {
                        self.pick_text_color = !self.pick_text_color;
                        if self.pick_text_color {
                            self.preprocess_target_extra = extra;
                        }
                    }
                    if !self.config.color_gate_points.is_empty()
                        && ui.small_button("用图色点位颜色").clicked()
                    {
                        new_color = Some(self.config.color_gate_points[0].color);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("色值");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut edit_value)
                            .desired_width(84.0)
                            .hint_text("#eaff65"),
                    );
                    if response.changed() {
                        if let Some(picked) = gate::parse_hex(&edit_value) {
                            new_color = Some(picked);
                        }
                    } else if !response.has_focus() {
                        edit_value = gate::to_hex(color);
                    }
                    if ui
                        .small_button("默认")
                        .on_hover_text("恢复默认颜色 234, 255, 101（#eaff65）")
                        .clicked()
                    {
                        new_color = Some(crate::config::DEFAULT_PREPROCESS_COLOR);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("容差");
                    changed |= ui
                        .add(egui::DragValue::new(&mut tolerance).range(0..=255))
                        .changed();
                });
                if tolerance > 80 {
                    ui.label(
                        egui::RichText::new(
                            "容差 >80 时颜色提取会退化成按亮度阈值：当前画面可能刚好清晰，\
                             但换地图或文字颜色就容易全黑 / 白字黑底。要稳定请直接用「二值化」。",
                        )
                        .small()
                        .color(theme::gold()),
                    );
                }
                ui.label(
                    egui::RichText::new(
                        "颜色提取只保留接近该颜色的像素并输出黑字白底：预览里若是白字黑底，说明选到的是背景色",
                    )
                    .small()
                    .weak(),
                );
                ui.collapsing("更多颜色", |ui| {
                    let mut color32 = Color32::from_rgb(color[0], color[1], color[2]);
                    if egui::color_picker::color_picker_color32(
                        ui,
                        &mut color32,
                        egui::color_picker::Alpha::Opaque,
                    ) {
                        new_color = Some([color32.r(), color32.g(), color32.b()]);
                    }
                });
                if let Some(picked) = new_color {
                    color = picked;
                    edit_value = gate::to_hex(picked);
                    changed = true;
                }
            }
            let summary = pipeline.summary();
            ui.label(
                egui::RichText::new(format!(
                    "顺序：{}（二值化已包含灰度，颜色提取自带反色；结果黑白颠倒时再勾反色）",
                    if summary == "关闭" {
                        "未启用"
                    } else {
                        &summary
                    }
                ))
                .small()
                .weak(),
            );
            let mode_value = pipeline.config_value();
            if changed || mode_value != previous_mode {
                if extra {
                    self.config.extra_preprocess_mode = mode_value;
                    self.config.extra_preprocess_color = color;
                    self.config.extra_preprocess_tolerance = tolerance;
                } else {
                    self.config.preprocess_mode = mode_value;
                    self.config.preprocess_color = color;
                    self.config.preprocess_tolerance = tolerance;
                }
                self.invalidate_scan();
                self.persist();
                self.refresh_preprocess_preview();
            }
            if extra {
                self.extra_preprocess_color_edit = edit_value;
            } else {
                self.preprocess_color_edit = edit_value;
            }
        });
    }

    pub(super) fn color_gate_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .checkbox(&mut self.config.color_gate_enabled, "启用")
                .changed()
            {
                self.invalidate_scan();
                self.check_gate();
                self.last_gate_check = Instant::now();
                self.persist();
            }
            ui.label("间隔");
            let mut interval = self.config.color_gate_interval_ms;
            if ui
                .add(
                    egui::DragValue::new(&mut interval)
                        .range(50..=5000)
                        .suffix(" ms"),
                )
                .changed()
            {
                self.config.color_gate_interval_ms = interval;
                self.persist();
            }
            ui.label("默认容差");
            let mut tolerance = self.config.color_gate_tolerance;
            if ui
                .add(egui::DragValue::new(&mut tolerance).range(0..=255))
                .changed()
            {
                self.config.color_gate_tolerance = tolerance;
                self.persist();
            }
            let mut mode = self.config.color_gate_mode.clone();
            let mut mode_changed = false;
            mode_changed |= ui
                .radio_value(&mut mode, "all".to_string(), "全部命中")
                .changed();
            mode_changed |= ui
                .radio_value(&mut mode, "any".to_string(), "任一命中")
                .changed();
            if mode_changed {
                self.config.color_gate_mode = mode;
                self.check_gate();
                self.persist();
            }
        });
        let status = if !self.config.color_gate_enabled {
            "未启用".to_string()
        } else if self.config.color_gate_points.is_empty() {
            "未配置点位（默认允许 OCR）".to_string()
        } else if self.gate_active {
            "有任务：允许 OCR".to_string()
        } else {
            "未命中：暂停 OCR".to_string()
        };
        ui.label(egui::RichText::new(format!("状态：{status}")).weak());

        let mut points = self.config.color_gate_points.clone();
        let gate_result = self.gate_result.clone();
        let mut changed = false;
        let mut remove = None;
        for (index, point) in points.iter_mut().enumerate() {
            ui.horizontal_wrapped(|ui| {
                let (swatch, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                ui.painter().rect_filled(
                    swatch,
                    CornerRadius::same(2),
                    Color32::from_rgb(point.color[0], point.color[1], point.color[2]),
                );
                ui.label(format!("({}, {}) {}", point.x, point.y, point.hex()));
                ui.label("容差");
                let mut tolerance = point.tolerance;
                if ui
                    .add(egui::DragValue::new(&mut tolerance).range(0..=255))
                    .changed()
                {
                    point.tolerance = tolerance;
                    changed = true;
                }
                ui.label("半径");
                let mut radius = point.radius;
                if ui
                    .add(egui::DragValue::new(&mut radius).range(0..=20))
                    .changed()
                {
                    point.radius = radius;
                    changed = true;
                }
                if let Some(result) = &gate_result {
                    if let (Some(hit), Some(sampled)) =
                        (result.matched.get(index), result.sampled.get(index))
                    {
                        let text = match sampled {
                            Some(color) => format!(
                                "实测 {} {}",
                                gate::to_hex(*color),
                                if *hit { "命中" } else { "未命中" }
                            ),
                            None => "越界".to_string(),
                        };
                        let color = if *hit {
                            Color32::from_rgb(120, 220, 120)
                        } else {
                            Color32::from_rgb(240, 160, 160)
                        };
                        ui.colored_label(color, text);
                    }
                }
                if ui.small_button("删除").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            points.remove(index);
            changed = true;
        }
        if ui.button("清空点位").clicked() {
            points.clear();
            changed = true;
        }
        if changed {
            self.config.color_gate_points = points;
            self.check_gate();
            self.last_gate_check = Instant::now();
            self.persist();
        }
    }

    /// 手动截取一次游戏客户区，只更新预览，不做 OCR。
    pub(super) fn capture_preview_once(&mut self) {
        if self.selected_handle.is_none() {
            self.refresh_windows(false);
        }
        let Some(handle) = self.selected_handle else {
            self.notice = Some("未找到游戏窗口，请打开游戏后刷新窗口列表".into());
            return;
        };
        let extra = if self.config.extra_region_enabled {
            Some(self.config.extra_task_region)
        } else {
            None
        };
        let title = self
            .windows
            .iter()
            .find(|window| window.handle == handle)
            .map(|window| window.title.clone())
            .unwrap_or_else(|| format!("#{handle}"));
        match capture::capture_task_panel(handle, self.config.task_region, extra) {
            Ok((_panel, full)) => {
                self.update_preview(full);
                self.notice = None;
                self.log_detail(format!("手动截取截图：{title}"));
                // 立即刷新一次图色门控状态，方便在取点工具里看命中情况。
                self.check_gate();
            }
            Err(err) => {
                self.log_detail(format!("手动截图失败：{err}"));
                self.notice = Some(err.clone());
                if err.contains("关闭") || err.contains("最小化") {
                    self.set_window(None);
                }
            }
        }
    }
}

/// 左/上/右/下 四个百分比，两行两列对齐。
fn region_grid(ui: &mut egui::Ui, id: &str, region: &mut crate::core::TaskRegion) -> bool {
    let mut values = [
        region.0 * 100.0,
        region.1 * 100.0,
        region.2 * 100.0,
        region.3 * 100.0,
    ];
    let mut edited = false;
    egui::Grid::new(("region_grid", id))
        .num_columns(4)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (index, (name, value)) in ["左", "上", "右", "下"]
                .into_iter()
                .zip(values.iter_mut())
                .enumerate()
            {
                ui.label(egui::RichText::new(name).small().weak());
                edited |= ui
                    .add(
                        egui::DragValue::new(value)
                            .speed(0.2)
                            .range(0.0..=100.0)
                            .suffix("%"),
                    )
                    .changed();
                if index % 2 == 1 {
                    ui.end_row();
                }
            }
        });
    if !edited {
        return false;
    }
    *region = (
        values[0] / 100.0,
        values[1] / 100.0,
        values[2] / 100.0,
        values[3] / 100.0,
    );
    region.0 = region.0.clamp(0.0, 0.99);
    region.1 = region.1.clamp(0.0, 0.99);
    region.2 = region.2.clamp(region.0 + 0.01, 1.0);
    region.3 = region.3.clamp(region.1 + 0.01, 1.0);
    true
}
/// 带标题的区域设置（标题单独一行）。
fn region_fields(
    ui: &mut egui::Ui,
    id: &str,
    title: &str,
    region: &mut crate::core::TaskRegion,
) -> bool {
    ui.label(egui::RichText::new(title).strong());
    region_grid(ui, id, region)
}
/// 取点窗口优先放在主窗口右侧，右侧空间不够就改放左侧；两边都放不下时才贴屏幕右缘，
/// 此时不可避免会和主窗口重叠，靠窗口层级（置顶）保证它显示在主窗口之上。
fn region_window_position(ctx: &egui::Context, size: Vec2) -> egui::Pos2 {
    ctx.input(|input| {
        let viewport = input.viewport();
        let monitor = viewport
            .monitor_size
            .unwrap_or_else(|| Vec2::new(1920.0, 1080.0));
        match viewport.outer_rect {
            Some(main) => place_region_window(main, monitor, size),
            // 拿不到主窗口位置时退化成屏幕右上角，至少保证整窗可见。
            None => egui::pos2((monitor.x - size.x).max(0.0), 0.0),
        }
    })
}
/// [`region_window_position`] 的纯计算部分：主窗口外框 + 显示器尺寸 → 取点窗口外框左上角。
pub(super) fn place_region_window(main: Rect, monitor: Vec2, size: Vec2) -> egui::Pos2 {
    const GAP: f32 = 8.0;
    // with_position 给的是窗口外框左上角，多留一个标题栏高度，免得下沿出屏。
    const TITLE_BAR: f32 = 32.0;
    let outer = size + Vec2::new(0.0, TITLE_BAR);
    let max_x = (monitor.x - outer.x).max(0.0);
    let max_y = (monitor.y - outer.y).max(0.0);
    let right = main.max.x + GAP;
    let left = main.min.x - GAP - outer.x;
    let x = if right <= max_x {
        right
    } else if left >= 0.0 {
        left
    } else {
        max_x
    };
    let y = main.min.y.clamp(0.0, max_y);
    egui::pos2(x, y)
}
/// 按可用宽高等比绘制一张预览纹理；没有纹理时显示提示文字。
fn preview_texture(
    ui: &mut egui::Ui,
    texture: &Option<TextureHandle>,
    max_width: f32,
    max_height: f32,
    empty_text: &str,
) {
    match texture {
        Some(texture) => {
            let size = texture.size_vec2();
            let scale = (max_width / size.x).min(max_height / size.y).min(1.0);
            let (rect, _) = ui.allocate_exact_size(size * scale, Sense::hover());
            ui.painter().image(
                texture.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            ui.label(egui::RichText::new(empty_text).small().weak());
        }
    }
}

/// 取点窗口的默认尺寸。
pub(super) const REGION_WINDOW_SIZE: Vec2 = Vec2::new(1000.0, 820.0);

pub(super) fn sample_image(image: &image::RgbImage, x: i32, y: i32) -> Option<[u8; 3]> {
    if x < 0 || y < 0 || x as u32 >= image.width() || y as u32 >= image.height() {
        return None;
    }
    let pixel = image.get_pixel(x as u32, y as u32);
    Some([pixel[0], pixel[1], pixel[2]])
}
