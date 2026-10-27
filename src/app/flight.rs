//! 合成旗点位编辑与标志样式界面。
use super::*;

impl MhxyApp {
    pub(super) fn save_flight_flags(&mut self) -> bool {
        match self.flight_flags.save(&self.flight_flags_path) {
            Ok(()) => {
                self.flight_feedback = Some(Ok("飞行旗点位已保存".to_string()));
                true
            }
            Err(err) => {
                self.flight_feedback = Some(Err(err));
                false
            }
        }
    }

    pub(super) fn flight_flags_ui(&mut self, ui: &mut egui::Ui) {
        let flag_count: usize = SUPPORTED_MAPS
            .iter()
            .map(|map| self.flight_flags.flags_for(map).len())
            .sum();
        ui.label(
            egui::RichText::new(format!("已配置 {flag_count} 面合成旗"))
                .small()
                .weak(),
        );
        if ui
            .add_sized(
                [ui.available_width(), 32.0],
                egui::Button::new(if self.show_flight_flags_window {
                    "关闭合成旗编辑器"
                } else {
                    "打开合成旗编辑器"
                }),
            )
            .clicked()
        {
            self.show_flight_flags_window = !self.show_flight_flags_window;
        }
        ui.label(
            egui::RichText::new(format!("本地文件：{}", self.flight_flags_path.display()))
                .small()
                .weak(),
        );
    }

    pub(super) fn flight_flags_editor_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("地图");
            let previous = self.flight_map_edit.clone();
            egui::ComboBox::from_id_salt("flight_flag_editor_map")
                .selected_text(&self.flight_map_edit)
                .show_ui(ui, |ui| {
                    for map in SUPPORTED_MAPS {
                        ui.selectable_value(&mut self.flight_map_edit, map.to_string(), map);
                    }
                });
            if previous != self.flight_map_edit {
                self.flight_flag_index = 0;
                self.flight_feedback = None;
            }
            ui.separator();
            ui.label("新建合成旗");
            ui.add(
                egui::TextEdit::singleline(&mut self.flight_flag_name_edit)
                    .desired_width(140.0)
                    .hint_text("例如：酒店旗"),
            );
            if ui.button("新建").clicked() {
                match self
                    .flight_flags
                    .add_flag(&self.flight_map_edit, &self.flight_flag_name_edit)
                {
                    Ok(index) => {
                        self.flight_flag_index = index;
                        self.flight_flag_name_edit.clear();
                        self.save_flight_flags();
                    }
                    Err(err) => self.flight_feedback = Some(Err(err)),
                }
            }
        });

        let flag_names: Vec<String> = self
            .flight_flags
            .flags_for(&self.flight_map_edit)
            .iter()
            .map(|flag| flag.name.clone())
            .collect();
        if self.flight_flag_index >= flag_names.len() && !flag_names.is_empty() {
            self.flight_flag_index = flag_names.len() - 1;
        }

        ui.horizontal_wrapped(|ui| {
            ui.label("当前合成旗");
            for (index, name) in flag_names.iter().enumerate() {
                ui.selectable_value(
                    &mut self.flight_flag_index,
                    index,
                    format!("{} · {name}", index + 1),
                );
            }
            if !flag_names.is_empty() && ui.button("删除当前旗").clicked() {
                self.flight_flags
                    .remove_flag(&self.flight_map_edit, self.flight_flag_index);
                self.flight_flag_index = self.flight_flag_index.saturating_sub(1);
                self.save_flight_flags();
            }
        });
        self.flight_flag_style_ui(ui);

        if flag_names.is_empty() {
            ui.label(
                egui::RichText::new("先新建一面合成旗，再点击地图添加点位").color(theme::gold()),
            );
        }
        let definition = self
            .maps
            .iter()
            .find(|map| map.name == self.flight_map_edit)
            .cloned();
        if let Some(definition) = definition {
            self.flight_flag_map_editor(ui, &definition, !flag_names.is_empty());
        } else {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("当前版本缺少长安城地图资源，暂时无法点击取点")
                    .color(theme::gold()),
            );
        }

        let points = self
            .flight_flags
            .flags_for(&self.flight_map_edit)
            .get(self.flight_flag_index)
            .map(|flag| flag.points.clone())
            .unwrap_or_default();
        ui.separator();
        ui.label(egui::RichText::new("当前合成旗点位").strong());
        let mut remove = None;
        ui.horizontal_wrapped(|ui| {
            for (index, point) in points.iter().enumerate() {
                if ui
                    .button(format!("({}, {})  ×", point.x, point.y))
                    .on_hover_text("删除这个点位")
                    .clicked()
                {
                    remove = Some(index);
                }
            }
        });
        if let Some(index) = remove {
            self.flight_flags
                .remove_point(&self.flight_map_edit, self.flight_flag_index, index);
            self.save_flight_flags();
        }
        if let Some(result) = &self.flight_feedback {
            let (text, color) = match result {
                Ok(text) => (text, theme::jade()),
                Err(text) => (text, theme::red()),
            };
            ui.label(egui::RichText::new(text).small().color(color));
        }
    }

    /// 编辑当前选中合成旗自己的标注颜色；没选旗时提示先选择。
    pub(super) fn flight_flag_style_ui(&mut self, ui: &mut egui::Ui) {
        let map = self.flight_map_edit.clone();
        let index = self.flight_flag_index;
        let fallback = self.flight_flags.style;
        let mut changed = false;
        {
            let Some(flag) = self
                .flight_flags
                .maps
                .get_mut(&map)
                .and_then(|flags| flags.get_mut(index))
            else {
                ui.label(
                    egui::RichText::new("选择一面合成旗后，可以单独设置它的背景和文字颜色")
                        .small()
                        .weak(),
                );
                return;
            };
            let mut style = flag.style.unwrap_or(fallback);
            ui.horizontal_wrapped(|ui| {
                ui.label("标注样式");
                let mut background =
                    hsva_for_blend_picker(style.background, style.background_normal_alpha);
                if ui.color_edit_button_hsva(&mut background).changed() {
                    let (color, normal_alpha) =
                        split_blend_alpha(background, style.background_normal_alpha);
                    style.background = hsva_to_array(color);
                    style.background_normal_alpha = normal_alpha;
                    changed = true;
                }
                ui.label("背景");
                let mut text = hsva_for_blend_picker(style.text, style.text_normal_alpha);
                if ui.color_edit_button_hsva(&mut text).changed() {
                    let (color, normal_alpha) = split_blend_alpha(text, style.text_normal_alpha);
                    style.text = hsva_to_array(color);
                    style.text_normal_alpha = normal_alpha;
                    changed = true;
                }
                ui.label("文字");
                let mut size = f32::from(style.size);
                if ui
                    .add(egui::Slider::new(&mut size, 12.0..=40.0).text("大小"))
                    .changed()
                {
                    style.size = size.round() as u8;
                    changed = true;
                }
            });
            if changed {
                flag.style = Some(style);
            }
        }
        if changed {
            self.save_flight_flags();
        }
    }

    pub(super) fn flight_flag_map_editor(
        &mut self,
        ui: &mut egui::Ui,
        definition: &MapDefinition,
        can_add: bool,
    ) {
        let Some(texture) = self.map_texture(ui.ctx(), definition) else {
            ui.colored_label(theme::gold(), "地图图片未配置");
            return;
        };
        let texture_size = texture.size_vec2();
        let available = vec2(
            ui.available_width(),
            (ui.available_height() - 100.0).max(260.0),
        );
        let (rect, response) = ui.allocate_exact_size(available, Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(8), theme::inset());
        let image_rect = fit_rect(rect.shrink(8.0), texture_size);
        painter.image(
            texture.id(),
            image_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let scale = image_rect.width() / texture_size.x;
        let flags = self.flight_flags.flags_for(&definition.name).to_vec();
        for (flag_index, flag) in flags.iter().enumerate() {
            for point in &flag.points {
                if let Some(pixel) = map_point(
                    definition,
                    point.x as f32,
                    point.y as f32,
                    texture_size.x,
                    texture_size.y,
                ) {
                    draw_flight_flag_marker(
                        &painter,
                        pos2(
                            image_rect.min.x + pixel.0 * scale,
                            image_rect.min.y + pixel.1 * scale,
                        ),
                        flag_index + 1,
                        flag_index == self.flight_flag_index,
                        self.flight_flags.flag_style(&definition.name, flag_index),
                    );
                }
            }
        }
        if can_add && response.clicked() {
            if let Some(pointer) = response
                .interact_pointer_pos()
                .filter(|p| image_rect.contains(*p))
            {
                let pixel_x = (pointer.x - image_rect.min.x) / scale;
                let pixel_y = (pointer.y - image_rect.min.y) / scale;
                if let Some((x, y)) =
                    map_coordinate(definition, pixel_x, pixel_y, texture_size.x, texture_size.y)
                {
                    match self.flight_flags.add_point(
                        &self.flight_map_edit,
                        self.flight_flag_index,
                        FlightPoint { x, y },
                    ) {
                        Ok(()) => {
                            self.save_flight_flags();
                        }
                        Err(err) => self.flight_feedback = Some(Err(err)),
                    }
                }
            }
        }
    }
}

pub(super) fn hsva_for_blend_picker(
    [h, s, v, a]: [f32; 4],
    normal_alpha: f32,
) -> egui::ecolor::Hsva {
    let additive = a.is_sign_negative();
    let alpha = if additive {
        -1.0
    } else {
        normal_alpha.max(f32::MIN_POSITIVE)
    };
    egui::ecolor::Hsva::new(h, s, v, alpha)
}

pub(super) fn split_blend_alpha(
    mut hsva: egui::ecolor::Hsva,
    previous_normal_alpha: f32,
) -> (egui::ecolor::Hsva, f32) {
    let additive = hsva.a.is_sign_negative();
    let normal_alpha = if additive {
        previous_normal_alpha
    } else {
        hsva.a
    };
    hsva.a = if additive { -1.0 } else { normal_alpha };
    (hsva, normal_alpha)
}

pub(super) fn marker_color([h, s, v, a]: [f32; 4], normal_alpha: f32) -> Color32 {
    let alpha = if a.is_sign_negative() {
        1.0
    } else {
        normal_alpha
    };
    Color32::from(egui::ecolor::Hsva::new(h, s, v, alpha))
}

pub(super) fn hsva_to_array(hsva: egui::ecolor::Hsva) -> [f32; 4] {
    [hsva.h, hsva.s, hsva.v, hsva.a]
}

pub(super) fn draw_flight_flag_marker(
    painter: &egui::Painter,
    center: egui::Pos2,
    number: usize,
    selected: bool,
    style: FlightFlagStyle,
) {
    // 只画背景圆和文字，不再描边；选中时稍微放大一点做区分。
    let radius = f32::from(style.size) * 0.5 + if selected { 2.0 } else { 0.0 };
    painter.circle_filled(
        center,
        radius,
        marker_color(style.background, style.background_normal_alpha),
    );
    painter.text(
        center,
        Align2::CENTER_CENTER,
        number.to_string(),
        FontId::proportional((f32::from(style.size) * 0.65).max(9.0)),
        marker_color(style.text, style.text_normal_alpha),
    );
}
