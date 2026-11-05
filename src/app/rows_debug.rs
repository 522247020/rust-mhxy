//! 行分割调试预览。
//!
//! 分行是新 OCR 主路径的第一步：把任务面板按「行」切开再逐行识别。
//! 切错了后面全错，所以这个窗口把每一行画出来，方便肉眼确认：
//! - 蓝色框 = 行带范围；编号 = 行序（从上到下）；
//! - 右侧竖条 = 行投影曲线（墨迹越多越长），能直接看出「谷」在哪；
//! - 下方 = 每行实际送识别的图（含留白）+ 这一行识别出来的文本。
//!
//! 这里只做展示，分行本身在 `crate::text_layout`，主程序在同一条路径上跑。
//! 二值图、投影曲线、行裁图都在换面板时算一次并缓存：窗口常开时不会每帧重算 Otsu。
use super::*;
use crate::text_layout::{self, RowLayout, RowSource};

/// 调试窗口里每行缩略图的最大宽度。
const ROW_THUMB_MAX_WIDTH: f32 = 620.0;
/// 行缩略图最多显示多少行（任务栏抓鬼+鬼王叠加可到十几行，缩略图进 ScrollArea，不设小上限）。
const MAX_ROW_THUMBS: usize = 64;

impl MhxyApp {
    /// 打开调试页并先抓一帧，避免空白。
    pub(super) fn open_rows_debug(&mut self) {
        self.sidebar_tab = "行分割调试";
        self.refresh_rows_debug();
    }

    /// 重新抓一帧面板并重新分行。
    pub(super) fn refresh_rows_debug(&mut self) {
        let Some(handle) = self.selected_handle else {
            self.rows_debug_message = Some("还没有选择游戏窗口".to_string());
            return;
        };
        let region = self.config.task_region;
        let extra = self.config.extra_task_region;
        let extra_enabled = self.config.extra_region_enabled;
        // 截图是阻塞调用，但只在点按钮 / 打开窗口时做一次，可以接受。
        let result = (|| -> Result<image::RgbImage, String> {
            let full = capture::capture_client(handle, region)?;
            let mut panels = vec![capture::crop_task_region(&full, region)];
            if extra_enabled {
                panels.push(capture::crop_task_region(&full, extra));
            }
            Ok(capture::stack_images(&panels))
        })();
        match result {
            Ok(panel) => self.set_rows_debug_panel(panel),
            Err(err) => self.rows_debug_message = Some(err),
        }
    }

    /// 在当前面板上用「当前设置」跑一次识别，逐行文本会写回预览：格子/留白对不对一眼看出。
    pub(super) fn test_rows_debug_ocr(&mut self) {
        let Some(panel) = self.rows_debug_panel.clone() else {
            self.rows_debug_message = Some("先抓一帧面板（抓取当前面板）".to_string());
            return;
        };
        // 设置改过就先按当前设置重算布局，保证行和识别结果一致。
        self.set_rows_debug_panel(panel.clone());
        self.rows_debug_test_running = true;
        self.rows_debug_message = None;
        // 调试页的「测试当前设置」按预览走：全局关闭分行时也要能对比签名效果，
        // 否则这里测的是检测模型，逐行文本无从对照。
        let mut row = self.row_tuning();
        if row.mode == crate::ocr::RowMode::Detect {
            row.mode = if self.rows_debug_row_config().line_height.is_some() {
                crate::ocr::RowMode::Manual
            } else {
                crate::ocr::RowMode::Auto
            };
        }
        let upscale = self.config.ocr_upscale;
        let result = self.worker.submit_test(panel, upscale, row);
        if let Err(err) = result {
            self.rows_debug_test_running = false;
            self.rows_debug_message = Some(err);
        }
    }

    /// 调试预览用的分行配置：手动参数只在手动网格下带（预览不受「关闭分行」影响，
    /// 关了分行也能看到到底能切成几行）。
    fn rows_debug_row_config(&self) -> text_layout::RowConfig {
        let manual = self.config.ocr_row_mode == crate::ocr::RowMode::Manual.config_value()
            && self.config.ocr_row_height >= 4;
        text_layout::RowConfig {
            line_height: manual.then_some(self.config.ocr_row_height),
            first_line_top: manual.then_some(self.config.ocr_row_start),
            gap_rows: if manual {
                text_layout::parse_gap_rows(&self.config.ocr_gap_rows)
            } else {
                Vec::new()
            },
            pad_ratio: self.config.ocr_row_pad as f32 / 100.0,
            ..Default::default()
        }
    }

    /// 用给定面板刷新调试数据（识别流程拿到新截图时也会走这里）。
    pub(super) fn set_rows_debug_panel(&mut self, panel: image::RgbImage) {
        let config = self.rows_debug_row_config();
        let layout = text_layout::rows(&panel, config.clone());
        // 一次算好：二值图、投影曲线、每行裁图。
        let binary = text_layout::binary(&panel);
        let profile = ink_profile(&binary);
        let rows = text_layout::row_images(&panel, &layout, config);
        self.rows_debug_message = None;
        self.rows_debug_layout = Some(layout);
        self.rows_debug_panel = Some(panel);
        self.rows_debug_binary = Some(binary);
        self.rows_debug_profile = profile;
        self.rows_debug_rows = rows;
        self.rows_debug_revision = self.rows_debug_revision.wrapping_add(1);
    }

    /// 记录最近一次识别给出的每行文本，调试窗口里直接对照。
    pub(super) fn set_rows_debug_texts(&mut self, layout: Option<&RowLayout>, texts: &[String]) {
        if self.rows_debug_layout.is_some() {
            self.rows_debug_row_texts = texts.to_vec();
            if let Some(layout) = layout {
                self.rows_debug_layout = Some(layout.clone());
            }
        }
    }

    /// 行分割调试页（主区）：标题 + 白卡（原浮窗）。
    pub(super) fn rows_debug_page_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(font) = theme::icon_font() {
                ui.label(
                    egui::RichText::new("\u{E8A5}")
                        .font(font)
                        .color(theme::jade()),
                );
            }
            ui.label(
                egui::RichText::new("行分割调试")
                    .size(20.0)
                    .color(theme::text())
                    .strong(),
            );
        });
        theme::divider(ui);
        theme::card_with(theme::surface()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.rows_debug_window_ui(ui);
        });
    }

    pub(super) fn rows_debug_window_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .add(theme::secondary_button("抓取当前面板"))
                .on_hover_text("按当前识别区域截一张图，并重新计算分行")
                .clicked()
            {
                self.refresh_rows_debug();
            }
            if ui.add(theme::secondary_button("重新分行")).clicked() {
                if let Some(panel) = self.rows_debug_panel.clone() {
                    self.set_rows_debug_panel(panel);
                }
            }
            let test_clicked = ui
                .add(theme::secondary_button("测试当前设置"))
                .on_hover_text("用当前行高/首行定位/留白/空白行设置跑一次识别，逐行文本写回预览")
                .clicked();
            if test_clicked {
                self.test_rows_debug_ocr();
            }
            if self.rows_debug_test_running {
                ui.label(egui::RichText::new("识别中…").small().color(theme::gold()));
            }
            ui.checkbox(&mut self.rows_debug_show_binary, "显示二值图");
        });
        // 手动行高 + 首行定位：手动网格，第一行从这个 Y 开始按行高往下切。
        let (previous_height, previous_start, previous_pad, previous_mode) = (
            self.config.ocr_row_height,
            self.config.ocr_row_start,
            self.config.ocr_row_pad,
            self.config.ocr_row_mode.clone(),
        );
        ui.horizontal(|ui| {
            ui.label("行高");
            let mut manual = crate::ocr::RowMode::from_config(&self.config.ocr_row_mode)
                == crate::ocr::RowMode::Manual;
            let auto_clicked = ui.selectable_label(!manual, "自动").clicked();
            let manual_clicked = ui.selectable_label(manual, "手动").clicked();
            // 这两个档位就是「识别方式 → 行分割」的自动/手动，点一下就写回配置。
            if auto_clicked {
                manual = false;
                self.config.ocr_row_mode = crate::ocr::RowMode::Auto.config_value().to_string();
            } else if manual_clicked {
                manual = true;
                self.config.ocr_row_mode = crate::ocr::RowMode::Manual.config_value().to_string();
            }
            if manual {
                if self.config.ocr_row_height < 4 {
                    self.config.ocr_row_height = self
                        .rows_debug_layout
                        .as_ref()
                        .map(|layout| layout.line_height)
                        .unwrap_or(16);
                }
                let mut height = self.config.ocr_row_height;
                if ui
                    .add(
                        egui::DragValue::new(&mut height)
                            .range(4..=200)
                            .suffix(" px"),
                    )
                    .on_hover_text("相邻两行起点的距离（行间距）；拖一下立即重新分行")
                    .changed()
                {
                    self.config.ocr_row_height = height;
                }
                ui.label("首行定位");
                let mut start = self.config.ocr_row_start;
                if ui
                    .add(
                        egui::DragValue::new(&mut start)
                            .range(0..=2000)
                            .suffix(" px"),
                    )
                    .on_hover_text("第一行从图片顶部往下第几个像素开始；想从图片顶部开始就填 0")
                    .changed()
                {
                    self.config.ocr_row_start = start;
                }
                ui.label("留白");
                let mut pad = self.config.ocr_row_pad;
                if ui
                    .add(egui::DragValue::new(&mut pad).range(0..=50).suffix(" %"))
                    .on_hover_text(
                        "行图上下各补白边=行高×留白%（白色，不含邻行像素）；设 0 就严格按行高切",
                    )
                    .changed()
                {
                    self.config.ocr_row_pad = pad;
                }
                ui.label(
                    egui::RichText::new(
                        "拖数字：调整后立刻重新分行。改动会写入配置，正式识别也按这个网格走。",
                    )
                    .small()
                    .weak(),
                );
            }
        });
        if (
            self.config.ocr_row_height,
            self.config.ocr_row_start,
            self.config.ocr_row_pad,
        ) != (previous_height, previous_start, previous_pad)
            || self.config.ocr_row_mode != previous_mode
        {
            self.persist();
            if let Some(panel) = self.rows_debug_panel.clone() {
                self.set_rows_debug_panel(panel);
            }
        }
        // 空白行：点「添加」就插入一行（随手给个高度），
        // 之后直接拖这行的高度，别再走「设数值 → 添加」试错。
        let previous_gap_rows = self.config.ocr_gap_rows.clone();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("空白行").strong());
            if ui
                .add(theme::secondary_button("添加空白行"))
                .on_hover_text("空白行只占高度，不识别；下面行会从空白行之后重新锚定对齐")
                .clicked()
            {
                let mut gaps = text_layout::parse_gap_rows(&self.config.ocr_gap_rows);
                let after = self
                    .rows_debug_layout
                    .as_ref()
                    .map(|layout| layout.rows.len())
                    .unwrap_or(1)
                    .max(1);
                let height = self.config.ocr_row_height.max(4);
                gaps.push((after, height));
                gaps.sort_by_key(|entry| entry.0);
                self.config.ocr_gap_rows = text_layout::serialize_gap_rows(&gaps);
            }
        });
        let mut gaps = text_layout::parse_gap_rows(&self.config.ocr_gap_rows);
        let mut deleted: Option<usize> = None;
        for (index, (after, height)) in gaps.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let (after, height) = (after, height);
                ui.label("第");
                let mut after_v = *after;
                if ui
                    .add(
                        egui::DragValue::new(&mut after_v)
                            .range(1..=100)
                            .suffix(" 行"),
                    )
                    .changed()
                {
                    *after = after_v;
                }
                ui.label("之后 ·");
                let mut height_v = *height;
                if ui
                    .add(
                        egui::DragValue::new(&mut height_v)
                            .range(1..=2000)
                            .suffix(" px"),
                    )
                    .on_hover_text("拖它：下面行马上按这个高度重新对齐")
                    .changed()
                {
                    *height = height_v.max(1);
                }
                if ui.add(theme::secondary_button("删")).clicked() {
                    deleted = Some(index);
                }
            });
        }
        if let Some(index) = deleted {
            gaps.remove(index);
        }
        // 写回去时顺便归一化（parse 会排序并合并同行号）。
        let canonical = text_layout::parse_gap_rows(&text_layout::serialize_gap_rows(&gaps));
        self.config.ocr_gap_rows = text_layout::serialize_gap_rows(&canonical);
        if self.config.ocr_gap_rows != previous_gap_rows {
            self.persist();
            if let Some(panel) = self.rows_debug_panel.clone() {
                self.set_rows_debug_panel(panel);
            }
        }
        theme::divider(ui);

        if let Some(message) = &self.rows_debug_message {
            ui.label(egui::RichText::new(message).small().color(theme::red()));
        }

        let Some(panel) = self.rows_debug_panel.clone() else {
            ui.label(egui::RichText::new("还没有截图：点「抓取当前面板」，或先跑一次识别").weak());
            return;
        };
        let Some(layout) = self.rows_debug_layout.clone() else {
            return;
        };

        // 统计行：一眼看出「切了几行、可不可信、走的哪条策略」。
        let source = match layout.source {
            RowSource::Projection => "投影",
            RowSource::Cluster => "聚类兜底",
        };
        let color = if layout.reliable {
            theme::jade()
        } else {
            theme::gold()
        };
        ui.label(
            egui::RichText::new(format!(
                "行数 {}｜行高 {}px｜策略 {}｜{}",
                layout.rows.len(),
                layout.line_height,
                source,
                if layout.reliable {
                    "可信（走分行识别）"
                } else {
                    "不可信（回退检测模型）"
                }
            ))
            .small()
            .color(color),
        );
        if self.config.ocr_row_mode == crate::ocr::RowMode::Manual.config_value() {
            ui.label(
                egui::RichText::new("当前走手动网格：首行定位与行高都是你定的")
                    .small()
                    .weak(),
            );
        } else if self.config.ocr_row_mode == crate::ocr::RowMode::Detect.config_value() {
            ui.label(
                egui::RichText::new(
                    "行分割已关闭（识别方式 → 行分割）：这里仍显示预览，正式识别直接走检测模型",
                )
                .small()
                .color(theme::gold()),
            );
        }
        for note in &layout.notes {
            ui.label(egui::RichText::new(format!("· {note}")).small().weak());
        }

        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("rows_debug_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.rows_debug_preview(ui, &panel, &layout);
                ui.add_space(6.0);
                self.rows_debug_thumbs(ui, &layout);
            });
    }

    /// 面板预览 + 行框 + 投影曲线。
    fn rows_debug_preview(
        &mut self,
        ui: &mut egui::Ui,
        panel: &image::RgbImage,
        layout: &RowLayout,
    ) {
        // 二值视图直接复用缓存的二值图，不每帧重算。
        let color = match (self.rows_debug_show_binary, self.rows_debug_binary.as_ref()) {
            (true, Some(binary)) => egui::ColorImage::from_gray(
                [binary.width() as usize, binary.height() as usize],
                binary.as_raw(),
            ),
            _ => to_color_image(panel),
        };
        let needs_upload = self.rows_debug_preview_revision != Some(self.rows_debug_revision);
        match &mut self.rows_debug_texture {
            Some(texture) => {
                if needs_upload {
                    texture.set(color, egui::TextureOptions::NEAREST);
                }
            }
            slot => {
                *slot = Some(self.ctx.load_texture(
                    "rows_debug_panel",
                    color,
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
        self.rows_debug_preview_revision = Some(self.rows_debug_revision);
        let Some(texture) = self.rows_debug_texture.clone() else {
            return;
        };

        // 预览宽度（面板 + 右侧投影条），按可用宽度等比缩放。
        let available = (ui.available_width() - 8.0).max(160.0);
        let profile_width = 72.0;
        let image_area = (available - profile_width).max(80.0);
        let scale = (image_area / panel.width().max(1) as f32).min(4.0);
        let size = vec2(panel.width() as f32 * scale, panel.height() as f32 * scale);
        let (rect, _) = ui.allocate_exact_size(vec2(available, size.y.max(40.0)), Sense::hover());
        let image_rect = Rect::from_min_size(rect.min, size);
        let painter = ui.painter_at(rect.expand(2.0));
        painter.rect_filled(image_rect, CornerRadius::same(4), theme::inset());
        painter.image(
            texture.id(),
            image_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );

        let row_color = theme::jade();
        for (index, row) in layout.rows.iter().enumerate() {
            let y0 = image_rect.top() + row.rect.y0 as f32 * scale;
            let y1 = image_rect.top() + row.rect.y1 as f32 * scale;
            let band = Rect::from_min_max(
                pos2(image_rect.left(), y0),
                pos2(image_rect.right(), y1.max(y0 + 1.0)),
            );
            painter.rect_stroke(
                band,
                CornerRadius::ZERO,
                egui::Stroke::new(1.5, row_color),
                egui::StrokeKind::Outside,
            );
            painter.text(
                pos2(image_rect.left() + 3.0, y0 + 1.0),
                Align2::LEFT_TOP,
                format!("#{}", index + 1),
                FontId::proportional(11.0),
                row_color,
            );
        }

        // 右侧投影曲线：横向长度 = 该 y 上的墨迹量，能直接看出行与行之间的谷。
        let profile_rect = Rect::from_min_max(
            pos2(image_rect.right() + 6.0, image_rect.top()),
            pos2(rect.right(), image_rect.bottom()),
        );
        painter.rect_filled(profile_rect, CornerRadius::same(2), theme::inset());
        let max_ink = self
            .rows_debug_profile
            .iter()
            .copied()
            .max()
            .unwrap_or(0)
            .max(1) as f32;
        let panel_height = panel.height().max(1) as f32;
        for (y, value) in self.rows_debug_profile.iter().enumerate() {
            let width_px = *value as f32 / max_ink * profile_rect.width();
            if width_px <= 0.0 {
                continue;
            }
            let y_pos = profile_rect.top() + y as f32 / panel_height * profile_rect.height();
            painter.line_segment(
                [
                    pos2(profile_rect.left(), y_pos),
                    pos2(profile_rect.left() + width_px, y_pos),
                ],
                egui::Stroke::new(0.6, theme::muted()),
            );
        }
    }

    /// 每行的裁图 + 识别文本：确认「一行一个字都不少」。
    fn rows_debug_thumbs(&mut self, ui: &mut egui::Ui, layout: &RowLayout) {
        if self.rows_debug_rows.is_empty() {
            ui.label(egui::RichText::new("这一帧没有切出行").small().weak());
            return;
        }
        let stale = self.rows_debug_thumb_revision != Some(self.rows_debug_revision)
            || self.rows_debug_thumb_binary != self.rows_debug_show_binary
            || self
                .rows_debug_thumbnail
                .as_ref()
                .is_none_or(|textures| textures.len() != self.rows_debug_rows.len());
        if stale {
            let mut textures: Vec<Option<TextureHandle>> =
                Vec::with_capacity(self.rows_debug_rows.len());
            // 缩略图跟随「显示二值图」开关：开了就显示二值裁图，和预览一致；
            // 关掉是送识别的灰度裁图（实际知道模型吃的是什么）。
            let binary_source = if self.rows_debug_show_binary {
                self.rows_debug_binary.clone()
            } else {
                None
            };
            let pad_y = (text_layout::RowConfig::default().pad_ratio
                * layout.line_height.max(1) as f32)
                .round() as u32;
            for (index, row) in self.rows_debug_rows.iter().enumerate() {
                let color = match (binary_source.as_ref(), layout.rows.get(index)) {
                    (Some(binary), Some(row_rect)) => {
                        let rect = row_rect.rect;
                        let x0 = rect.x0.saturating_sub(2);
                        let x1 = (rect.x1 + 2).min(binary.width());
                        let y0 = rect.y0.saturating_sub(pad_y);
                        let y1 = (rect.y1 + pad_y).min(binary.height());
                        let crop =
                            image::imageops::crop_imm(binary, x0, y0, x1 - x0, y1 - y0).to_image();
                        egui::ColorImage::from_gray(
                            [crop.width() as usize, crop.height() as usize],
                            crop.as_raw(),
                        )
                    }
                    _ => to_color_image(row),
                };
                textures.push(Some(self.ctx.load_texture(
                    format!("rows_debug_row_{index}"),
                    color,
                    egui::TextureOptions::NEAREST,
                )));
            }
            self.rows_debug_thumbnail = Some(textures);
            self.rows_debug_thumb_revision = Some(self.rows_debug_revision);
            self.rows_debug_thumb_binary = self.rows_debug_show_binary;
        }

        for (index, row) in self.rows_debug_rows.iter().enumerate().take(MAX_ROW_THUMBS) {
            let text = self
                .rows_debug_row_texts
                .get(index)
                .cloned()
                .unwrap_or_default();
            let meta = layout
                .rows
                .get(index)
                .map(|row| format!("高 {}px｜墨迹 {}", row.rect.y1 - row.rect.y0, row.ink))
                .unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("#{}", index + 1))
                        .small()
                        .strong(),
                );
                ui.label(egui::RichText::new(meta).small().weak());
                if text.is_empty() {
                    ui.label(egui::RichText::new("（无文本）").small().weak());
                } else {
                    ui.label(egui::RichText::new(&text).small().color(theme::gold()));
                }
            });
            let texture = self
                .rows_debug_thumbnail
                .as_ref()
                .and_then(|textures| textures.get(index))
                .and_then(|texture| texture.as_ref());
            if let Some(texture) = texture {
                let scale = (ROW_THUMB_MAX_WIDTH / row.width().max(1) as f32).min(3.0);
                let size = vec2(
                    row.width() as f32 * scale,
                    (row.height() as f32 * scale).max(12.0),
                );
                ui.add(egui::Image::new((texture.id(), size)).sense(Sense::hover()));
            }
            ui.add_space(4.0);
        }
    }
}

/// 逐行墨迹数量（用于调试窗口里画投影曲线）。
fn ink_profile(binary: &image::GrayImage) -> Vec<u32> {
    let (width, height) = binary.dimensions();
    let mut out = vec![0u32; height as usize];
    for y in 0..height {
        let mut count = 0u32;
        for x in 0..width {
            if binary.get_pixel(x, y)[0] < 128 {
                count += 1;
            }
        }
        out[y as usize] = count;
    }
    out
}
