//! 任务卡片与地图渲染。
use super::flight::draw_flight_flag_marker;
use super::*;

impl MhxyApp {
    /// 把测试 OCR 结果整理成几行可读文本：耗时、识别文字、解析出的任务。
    pub(super) fn format_test_result(&self, text: &OcrText) -> String {
        let tasks = parse_tasks(
            &text.text,
            &self.maps,
            TaskFilter {
                zhuagui: true,
                ghost_king: true,
                generic: true,
            },
        );
        let mut lines = vec![format!("耗时 {:.2} 秒", text.elapsed_secs)];
        let raw = text.text.trim().replace('\n', " | ");
        if raw.is_empty() {
            lines.push("未识别到文字".to_string());
        } else {
            lines.push(format!("文字：{}", truncate_text(&raw, 240)));
        }
        if tasks.is_empty() {
            lines.push("未解析到任务（可检查截图区域或预处理）".to_string());
        }
        for task in tasks {
            lines.push(format!(
                "{} {} ({}, {}){}",
                task.task_type,
                task.map_name,
                task.x,
                task.y,
                if task.nearby { " 附近" } else { "" }
            ));
        }
        lines.join("\n")
    }
    pub(super) fn map_texture(
        &mut self,
        ctx: &egui::Context,
        definition: &MapDefinition,
    ) -> Option<TextureHandle> {
        let key = definition.image.to_string_lossy().to_string();
        if let Some(handle) = self.map_textures.get(&key) {
            return Some(handle.clone());
        }
        if self.map_texture_failed.contains(&key) {
            return None;
        }
        let image = match crate::core::load_rgb_image(&definition.image) {
            Ok(image) => image,
            Err(err) => {
                if self.map_texture_failed.insert(key.clone()) {
                    self.log_detail(format!(
                        "地图 {} 图片加载失败：{}（{err}）",
                        definition.name,
                        definition.image.display()
                    ));
                }
                return None;
            }
        };
        let color = to_color_image(&image);
        let handle = ctx.load_texture(key.clone(), color, egui::TextureOptions::LINEAR);
        self.map_textures.insert(key, handle.clone());
        Some(handle)
    }

    pub(super) fn render_map(
        &mut self,
        ui: &mut egui::Ui,
        definition: &MapDefinition,
        location: &TaskLocation,
        show_high_chance: bool,
    ) {
        let Some(texture) = self.map_texture(ui.ctx(), definition) else {
            ui.colored_label(Color32::from_rgb(240, 180, 80), "地图图片未配置");
            ui.add_space(6.0);
            return;
        };
        let texture_size = texture.size_vec2();
        let available = ui.available_size();
        if available.x < 8.0 || available.y < 8.0 {
            return;
        }
        let (rect, response) = ui.allocate_exact_size(available, Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(8), theme::inset());
        let outer = rect.shrink(8.0);
        let image_rect = fit_rect(outer, texture_size);
        if image_rect.width() <= 1.0 {
            return;
        }
        painter.image(
            texture.id(),
            image_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let scale = image_rect.width() / texture_size.x;
        let to_screen = |point: (f32, f32)| {
            pos2(
                image_rect.min.x + point.0 * scale,
                image_rect.min.y + point.1 * scale,
            )
        };
        let (point_x, point_y) = clamp_to_map(definition, location.x as f32, location.y as f32);

        // 高概率格子：相对任务坐标偏移的 50×50 区域。
        let high_chance_rects: Vec<Rect> = if show_high_chance {
            high_chance_areas(
                &definition.name,
                location.x as f32,
                location.y as f32,
                HIGH_CHANCE_SIZE,
            )
            .into_iter()
            .filter_map(|(chance_x, chance_y)| {
                let (left, top, right, bottom) = map_rect(
                    definition,
                    chance_x,
                    chance_y,
                    chance_x + HIGH_CHANCE_SIZE,
                    chance_y + HIGH_CHANCE_SIZE,
                    texture_size.x,
                    texture_size.y,
                );
                let chance = Rect::from_min_max(to_screen((left, top)), to_screen((right, bottom)))
                    .intersect(image_rect);
                (chance.width() > 0.0 && chance.height() > 0.0).then_some(chance)
            })
            .collect()
        } else {
            Vec::new()
        };

        let nearby_area: Option<Rect> = if location.nearby {
            // 用实际任务坐标算范围框（不是贴边后的 point_x/point_y），
            // 超出地图的部分由 map_rect 裁到地图边缘，这样黄框位置才和真实坐标对应。
            let (left, top, right, bottom) = map_area(
                definition,
                location.x as f32,
                location.y as f32,
                NEARBY_RADIUS * 2.0,
                texture_size.x,
                texture_size.y,
            );
            Some(
                Rect::from_min_max(to_screen((left, top)), to_screen((right, bottom)))
                    .intersect(image_rect),
            )
        } else {
            None
        };

        if let Some(area) = nearby_area {
            let shade = Color32::from_rgba_unmultiplied(0, 0, 0, 150);
            painter.rect_filled(
                Rect::from_min_max(image_rect.min, pos2(image_rect.max.x, area.min.y)),
                CornerRadius::ZERO,
                shade,
            );
            painter.rect_filled(
                Rect::from_min_max(pos2(image_rect.min.x, area.max.y), image_rect.max),
                CornerRadius::ZERO,
                shade,
            );
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(image_rect.min.x, area.min.y),
                    pos2(area.min.x, area.max.y),
                ),
                CornerRadius::ZERO,
                shade,
            );
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(area.max.x, area.min.y),
                    pos2(image_rect.max.x, area.max.y),
                ),
                CornerRadius::ZERO,
                shade,
            );
            painter.rect_stroke(
                area,
                CornerRadius::ZERO,
                egui::Stroke::new(2.0, Color32::from_rgb(255, 209, 102)),
                egui::StrokeKind::Inside,
            );
        }

        for chance in &high_chance_rects {
            // 只显示落在黄色「附近」框内的部分，避免红框超出黄框。
            let chance = match nearby_area {
                Some(area) => (*chance).intersect(area),
                None => *chance,
            };
            if chance.width() <= 0.0 || chance.height() <= 0.0 {
                continue;
            }
            painter.rect_filled(
                chance,
                CornerRadius::ZERO,
                Color32::from_rgba_unmultiplied(255, 40, 40, 48),
            );
            painter.rect_stroke(
                chance,
                CornerRadius::ZERO,
                egui::Stroke::new(2.0, Color32::from_rgba_unmultiplied(255, 40, 40, 170)),
                egui::StrokeKind::Inside,
            );
        }

        for (flag_index, flag) in self
            .flight_flags
            .flags_for(&definition.name)
            .iter()
            .enumerate()
        {
            for flight in &flag.points {
                if let Some(point) = map_point(
                    definition,
                    flight.x as f32,
                    flight.y as f32,
                    texture_size.x,
                    texture_size.y,
                ) {
                    draw_flight_flag_marker(
                        &painter,
                        to_screen(point),
                        flag_index + 1,
                        false,
                        self.flight_flags.flag_style(&definition.name, flag_index),
                    );
                }
            }
        }

        if let Some(point) = map_point(definition, point_x, point_y, texture_size.x, texture_size.y)
        {
            let center = to_screen(point);
            painter.circle_filled(center, 8.0, Color32::from_rgb(255, 51, 78));
            painter.circle_stroke(center, 8.0, egui::Stroke::new(2.0, Color32::WHITE));
            let label = format!("({}, {})", location.x, location.y);
            let galley = painter.layout_no_wrap(label, FontId::proportional(14.0), Color32::WHITE);
            let label_size = galley.size() + vec2(12.0, 8.0);
            let label_min = pos2(
                (center.x + 12.0)
                    .clamp(rect.left(), (rect.right() - label_size.x).max(rect.left())),
                (center.y - 30.0).clamp(rect.top(), (rect.bottom() - label_size.y).max(rect.top())),
            );
            painter.rect_filled(
                Rect::from_min_size(label_min, label_size),
                CornerRadius::same(4),
                Color32::from_black_alpha(220),
            );
            painter.galley(label_min + vec2(6.0, 4.0), galley, Color32::WHITE);
        }

        if let Some(pointer) = response.hover_pos() {
            if image_rect.contains(pointer) {
                let pixel_x = (pointer.x - image_rect.min.x) / scale;
                let pixel_y = (pointer.y - image_rect.min.y) / scale;
                if let Some((game_x, game_y)) =
                    map_coordinate(definition, pixel_x, pixel_y, texture_size.x, texture_size.y)
                {
                    painter.text(
                        image_rect.right_top() + vec2(-8.0, 8.0),
                        Align2::RIGHT_TOP,
                        format!("鼠标坐标：{game_x}, {game_y}"),
                        FontId::proportional(13.0),
                        Color32::from_rgb(248, 250, 252),
                    );
                }
            }
        }
    }

    /// 任务区域只显示地图；地图名称、坐标和识别状态不再单独占一行。
    pub(super) fn task_panel(&mut self, ui: &mut egui::Ui, task_type: &str) {
        let (location, map_index) = {
            let state = self.tasks.get(task_type).expect("task state");
            (state.location.clone(), state.map_index)
        };
        let accent = if task_type == "抓鬼" {
            theme::jade()
        } else {
            theme::gold()
        };
        let show_high_chance = match task_type {
            "抓鬼" => self.config.high_chance,
            "鬼王" => self.config.ghost_king_high_chance,
            _ => false,
        };
        let compact = self.compact;
        let draw = |app: &mut Self, ui: &mut egui::Ui| {
            ui.set_min_size(ui.available_size());
            match (&location, map_index) {
                (Some(location), Some(index)) => {
                    let definition = app.maps[index].clone();
                    app.render_map(ui, &definition, location, show_high_chance);
                }
                _ => {
                    let size = ui.available_size().max(vec2(1.0, 1.0));
                    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(rect, CornerRadius::same(8), theme::inset());
                    let center = rect.center();
                    painter.circle_stroke(center, 18.0, egui::Stroke::new(1.0, theme::border()));
                    painter.circle_stroke(
                        center,
                        9.0,
                        egui::Stroke::new(1.0, accent.gamma_multiply(0.6)),
                    );
                    painter.line_segment(
                        [center - vec2(25.0, 0.0), center + vec2(25.0, 0.0)],
                        egui::Stroke::new(1.0, theme::border()),
                    );
                    painter.line_segment(
                        [center - vec2(0.0, 25.0), center + vec2(0.0, 25.0)],
                        egui::Stroke::new(1.0, theme::border()),
                    );
                }
            }
        };
        if compact {
            // 精简模式不套任务卡片的边框和背景，地图直接铺满窗口。
            draw(self, ui);
        } else {
            theme::card().show(ui, |ui| draw(self, ui));
        }
    }
}
