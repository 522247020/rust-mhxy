//! app 模块的渲染与行为测试。
use super::flight::{hsva_for_blend_picker, hsva_to_array, marker_color, split_blend_alpha};
use super::*;
use eframe::App;

fn app() -> MhxyApp {
    let ctx = egui::Context::default();
    theme::install(&ctx, false);
    MhxyApp::from_config(Config::default(), PathBuf::from("/unused/config.json"), ctx)
}

/// 收集这一帧渲染出的所有文字，用于断言哪些界面元素出现了。
fn collect_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// 收集这一帧每段文字的位置，用于断言顶栏按钮顺序。
fn text_positions(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Pos2)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
        match shape {
            egui::Shape::Text(text) => out.push((text.galley.text().to_string(), text.pos)),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

fn escape_input() -> egui::RawInput {
    let mut input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
        ..Default::default()
    };
    input.events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::default(),
    });
    input
}

#[test]
fn blend_mode_switching_preserves_normal_alpha_and_additive_markers_are_opaque() {
    let normal = hsva_for_blend_picker([0.2, 0.5, 0.7, 0.0], 0.0);
    assert_eq!(normal.a, f32::MIN_POSITIVE);
    let mut normal_for_picker = hsva_for_blend_picker(hsva_to_array(normal), 0.4);
    assert!(normal_for_picker.a > 0.0);
    assert_eq!(normal_for_picker.a, 0.4);

    normal_for_picker.a = -normal_for_picker.a.abs();
    let (additive, remembered_alpha) = split_blend_alpha(normal_for_picker, 0.4);
    assert!(additive.a.is_sign_negative());
    assert_eq!(remembered_alpha, 0.4);
    let additive_color = marker_color(hsva_to_array(additive), remembered_alpha);
    assert_eq!(additive_color.a(), 255);

    let mut switched_to_normal = additive;
    switched_to_normal.a = remembered_alpha;
    let (back_to_normal, restored_alpha) = split_blend_alpha(switched_to_normal, remembered_alpha);
    assert!(!back_to_normal.a.is_sign_negative());
    assert_eq!(restored_alpha, 0.4);
    assert_eq!(back_to_normal.a, 0.4);
    let normal_color = marker_color(hsva_to_array(back_to_normal), restored_alpha);
    assert_eq!(normal_color.a(), 102);

    let nonzero = hsva_for_blend_picker([0.2, 0.5, 0.7, 0.25], 0.25);
    assert_eq!(nonzero.a, 0.25);
    let opaque_additive = hsva_for_blend_picker(hsva_to_array(additive), remembered_alpha);
    assert_eq!(opaque_additive.a, -1.0);
}

#[test]
fn flight_flag_editor_lists_supported_maps_and_local_file() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(420.0, 600.0))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| app.flight_flags_ui(ui));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(
        texts.iter().any(|text| text == "打开合成旗编辑器"),
        "侧栏应提供独立编辑窗口入口：{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text.contains("flight_flags.json")),
        "编辑区应说明独立本地配置文件：{texts:?}"
    );

    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 760.0))),
            ..Default::default()
        },
        |ui| app.flight_flags_editor_ui(ui),
    );
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(texts.iter().any(|text| text == "新建合成旗"));
    assert!(texts
        .iter()
        .any(|text| text == "先新建一面合成旗，再点击地图添加点位"));
    assert!(
        texts.iter().any(|text| text.contains("选择一面合成旗后")),
        "没选合成旗时应提示先选择：{texts:?}"
    );

    // 选中一面旗后，样式区只保留背景和文字颜色，而且只作用于这面旗。
    app.flight_flags.add_flag("傲来国", "测试旗").unwrap();
    app.flight_map_edit = "傲来国".to_string();
    app.flight_flag_index = 0;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 760.0))),
            ..Default::default()
        },
        |ui| app.flight_flags_editor_ui(ui),
    );
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(
        texts.iter().any(|text| text == "背景"),
        "样式区应保留背景颜色：{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text == "文字"),
        "样式区应保留文字颜色：{texts:?}"
    );
    assert!(
        !texts.iter().any(|text| text == "边框"),
        "合成旗标注不再有边框设置：{texts:?}"
    );
}

#[test]
fn meditation_hotkey_restarts_instead_of_pausing() {
    let mut app = app();
    app.meditation.restart();
    app.meditation.pause();
    assert_eq!(app.meditation.state(), MeditationState::Paused);

    app.meditation_hotkey_tx.send(()).unwrap();
    app.poll_meditation_hotkey();

    assert_eq!(
        app.meditation.state(),
        MeditationState::Running,
        "快捷键应重新开始计时，而不是继续暂停"
    );
    assert!(
        app.meditation.remaining() > Duration::from_secs(170),
        "重新开始后剩余时间应接近完整时长"
    );
}

#[test]
fn ocr_hotkey_registers_while_running_in_auto_mode() {
    let mut app = app();
    app.config.trigger_mode = "auto".into();
    app.config.ocr_hotkey = "Ctrl+Alt+F23".into();
    app.running = true;
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
            ..Default::default()
        },
        |ui| app.ui(ui, &mut frame),
    );
    output.textures_delta.clear();
    assert_eq!(
        app.hotkey_attempted.as_deref(),
        Some("Ctrl+Alt+F23"),
        "自动模式下运行中也要注册手动识别快捷键"
    );
    assert!(
        app.hotkey_listener.is_some() || app.hotkey_error.is_some(),
        "不能停在未注册也没有错误的状态"
    );
}

#[test]
fn manual_trigger_is_queued_while_busy_instead_of_dropped() {
    let mut app = app();
    app.running = true;
    app.busy = true;
    app.manual_trigger();
    assert!(
        app.pending_manual_trigger,
        "识别进行中按快捷键应排队，而不是直接丢掉（丢掉看起来就像快捷键没反应）"
    );

    // 上一轮结束后排队的那一次要被消费掉。
    app.busy = false;
    app.flush_pending_manual_trigger();
    assert!(!app.pending_manual_trigger, "排队的触发应当被消费一次");
}

#[test]
fn manual_trigger_is_refused_before_running() {
    let mut app = app();
    app.running = false;
    app.manual_trigger();
    assert!(
        !app.pending_manual_trigger,
        "没开始识别时按下快捷键不应排队"
    );
}

#[test]
fn meditation_hotkey_registers_without_starting_ocr() {
    let mut app = app();
    app.config.meditation_enabled = true;
    app.config.meditation_hotkey = "Ctrl+Alt+F24".into();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
            ..Default::default()
        },
        |ui| app.ui(ui, &mut frame),
    );
    output.textures_delta.clear();
    assert_eq!(
        app.meditation_hotkey_attempted.as_deref(),
        Some("Ctrl+Alt+F24"),
        "启用打坐计时后应尝试注册快捷键，不需要先开始识别"
    );
    assert!(
        app.meditation_hotkey_listener.is_some() || app.meditation_hotkey_error.is_some(),
        "不能停留在「正在注册」状态"
    );
}

#[test]
fn meditation_panel_shows_countdown_and_controls() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 900.0))),
        ..Default::default()
    };
    // 侧栏不再有打坐分组，顶栏提供「打坐」按钮。
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(
        texts.iter().any(|text| text == "打坐"),
        "顶栏应有打坐按钮：{texts:?}"
    );
    assert!(
        !texts.iter().any(|text| text == "启用打坐计时"),
        "打坐设置不应再出现在侧栏：{texts:?}"
    );

    // 独立窗口里的设置项。
    let mut output = ctx.run_ui(input(), |ui| app.meditation_controls(ui));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(
        texts.iter().any(|text| text == "启用打坐计时"),
        "应有功能的启用开关：{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text == "开始"),
        "待开始时应显示开始按钮：{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text == "重置"),
        "应有重置按钮：{texts:?}"
    );
}

#[test]
fn basic_sidebar_keeps_primary_path_and_hides_advanced_details() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 900.0))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    let positions = text_positions(&output.shapes);
    output.textures_delta.clear();
    for expected in [
        "游戏窗口",
        "识别模式",
        "任务类型",
        "设置识别区域",
        "开始识别",
        "高级设置",
    ] {
        assert!(
            texts.iter().any(|text| text == expected),
            "基础侧栏缺少「{expected}」：{texts:?}"
        );
    }
    // 高级设置默认收起，低频项不应出现在首屏。
    for hidden in [
        "远端 PP-OCR",
        "服务地址",
        "访问令牌（可选）",
        "识别方案",
        "打坐计时",
        "合成旗标注",
        "日志",
    ] {
        assert!(
            !texts.iter().any(|text| text == hidden),
            "高级设置收起时不应显示「{hidden}」：{texts:?}"
        );
    }
    // 底部固定区始终给出当前窗口、识别方式和主操作。
    assert!(texts.iter().any(|text| text == "本地 PP-OCRv5"));
    assert!(texts.iter().any(|text| text.contains("每 5 秒自动识别")));

    let find = |needle: &str| {
        positions
            .iter()
            .find(|(text, _)| text == needle)
            .map(|(_, pos)| *pos)
            .unwrap_or_else(|| panic!("侧栏缺少「{needle}」：{positions:?}"))
    };
    let window = find("游戏窗口");
    let task = find("任务类型");
    let start = positions
        .iter()
        .filter(|(text, _)| text == "开始识别")
        .map(|(_, pos)| *pos)
        .find(|pos| pos.x < 420.0)
        .expect("侧栏底部应有开始识别按钮");
    assert!(
            window.y < task.y && task.y < start.y,
            "基础路径应为 游戏窗口 → 任务类型 → 开始识别：window={window:?} task={task:?} start={start:?}"
        );
}

#[test]
fn interval_is_only_shown_in_auto_mode() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(420.0, 900.0))),
        ..Default::default()
    };

    app.config.trigger_mode = "auto".into();
    let mut output = ctx.run_ui(input(), |ui| app.basic_controls(ui));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(
        texts.iter().any(|text| text == "识别间隔"),
        "自动模式应显示识别间隔：{texts:?}"
    );
    assert!(!texts.iter().any(|text| text == "快捷键"));

    app.config.trigger_mode = "manual".into();
    let mut output = ctx.run_ui(input(), |ui| app.basic_controls(ui));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(
        !texts.iter().any(|text| text == "识别间隔"),
        "手动模式不应占位显示识别间隔：{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text == "快捷键"),
        "手动模式应显示快捷键设置：{texts:?}"
    );
    assert!(texts.iter().any(|text| text == "立即识别一次"));
}

#[test]
fn sidebar_action_follows_trigger_mode_and_running() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(320.0, 240.0))),
        ..Default::default()
    };

    app.config.trigger_mode = "manual".into();
    app.running = false;
    let mut output = ctx.run_ui(input(), |ui| app.sidebar_action(ui));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(texts.iter().any(|text| text == "开始监听"));
    assert!(texts.iter().any(|text| text.contains("手动 · F8")));

    app.running = true;
    let mut output = ctx.run_ui(input(), |ui| app.sidebar_action(ui));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(texts.iter().any(|text| text == "暂停监听"));

    app.config.trigger_mode = "auto".into();
    app.config.interval_seconds = 5;
    app.running = false;
    let mut output = ctx.run_ui(input(), |ui| app.sidebar_action(ui));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(texts.iter().any(|text| text == "开始识别"));
    assert!(texts
        .iter()
        .any(|text| text == "本地 PP-OCRv5 · 每 5 秒自动识别"));
}

#[test]
fn advanced_settings_hold_low_frequency_controls() {
    let mut app = app();
    app.config.ocr_mode = "remote".into();
    app.config.api_url = "http://127.0.0.1:8008".into();
    app.config.color_gate_enabled = true;
    app.config.preprocess_mode = "binary".into();
    let ctx = app.ctx.clone();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 2600.0))),
            ..Default::default()
        },
        |ui| app.advanced_controls(ui),
    );
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    for expected in [
        "识别方式",
        "本地 PP-OCR",
        "远端 PP-OCR",
        "服务地址",
        "访问令牌（可选）",
        "测试连接",
        "方案管理",
        "识别方案",
        "合成旗标注",
        "打开合成旗编辑器",
    ] {
        assert!(
            texts.iter().any(|text| text.contains(expected)),
            "高级设置缺少「{expected}」：{texts:?}"
        );
    }
    // 识别区域/门控在基础区「设置识别区域」里；打坐有独立窗口，日志不再展示内容。
    for removed in [
        "识别区域、门控与预处理",
        "打开识别区域设置",
        "启用打坐计时",
        "暂无日志",
        "显示与布局",
        "显示高概率区域",
    ] {
        assert!(
            !texts.iter().any(|text| text.contains(removed)),
            "高级设置不应再包含「{removed}」：{texts:?}"
        );
    }

    // 切回本地后不再显示远端地址输入。
    app.config.ocr_mode = "local".into();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 900.0))),
            ..Default::default()
        },
        |ui| app.advanced_controls(ui),
    );
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(!texts.iter().any(|text| text == "服务地址"));
    assert!(texts.iter().any(|text| text.contains("首次识别时加载模型")));
}

#[test]
fn generic_task_is_exclusive_with_zhuagui_and_ghost_king() {
    let mut app = app();
    // 即使配置里三个都开着（旧配置或手改），通用任务也必须独占。
    app.config.generic_enabled = true;
    app.config.zhuagui_enabled = true;
    app.config.ghost_king_enabled = true;
    let filter = app.task_filter();
    assert!(filter.generic, "通用任务应生效");
    assert!(
        !filter.zhuagui && !filter.ghost_king,
        "通用任务不能与抓鬼/鬼王并存"
    );
    assert!(app.task_active("通用"));
    assert!(!app.task_active("抓鬼"));
    assert!(!app.task_active("鬼王"));
}

#[test]
fn profile_saves_and_restores_task_region() {
    let mut app = app();
    app.config.task_region = (0.11, 0.22, 0.33, 0.44);
    app.config.extra_task_region = (0.55, 0.66, 0.77, 0.88);
    app.config.extra_region_enabled = true;
    app.save_active_profile();

    app.config.task_region = crate::core::DEFAULT_TASK_REGION;
    app.config.extra_task_region = crate::core::DEFAULT_EXTRA_REGION;
    app.config.extra_region_enabled = false;
    let name = app.config.active_profile.clone();
    app.apply_profile(&name);

    assert_eq!(app.config.task_region, (0.11, 0.22, 0.33, 0.44));
    assert_eq!(app.config.extra_task_region, (0.55, 0.66, 0.77, 0.88));
    assert!(app.config.extra_region_enabled);
}

#[test]
fn task_panel_has_no_high_chance_checkbox() {
    let mut app = app();
    app.config.high_chance = true;
    app.config.ghost_king_high_chance = true;
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 900.0))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input.clone(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    assert!(
        !texts.iter().any(|text| text == "高概率区域"),
        "地图上方不应再有高概率勾选框：{texts:?}"
    );
    output.textures_delta.clear();

    // 高概率开关在基础区「显示与布局」里。
    let mut output = ctx.run_ui(input, |ui| app.basic_controls(ui));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(
        texts.iter().any(|text| text.contains("显示高概率区域")),
        "基础区应保留高概率开关：{texts:?}"
    );
}

#[test]
fn header_orders_update_help_theme_top_compact_start() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
    let positions = text_positions(&output.shapes);
    let find = |needle: &str| {
        positions
            .iter()
            .find(|(text, _)| text == needle)
            .map(|(_, pos)| *pos)
            .unwrap_or_else(|| panic!("顶栏缺少「{needle}」：{positions:?}"))
    };
    let update = find("检查更新");
    let help = find("说明");
    let theme = find("界面主题");
    let on_top = find("置顶");
    let compact = find("精简界面");
    let start = find("开始识别");
    assert!(
        update.x < help.x
            && help.x < theme.x
            && theme.x < on_top.x
            && on_top.x < compact.x
            && compact.x < start.x,
        "顶栏顺序应为 检查更新 | 说明 | 界面主题 | 置顶 | 精简界面 | 开始识别：\
             update={update:?} help={help:?} theme={theme:?} on_top={on_top:?} \
             compact={compact:?} start={start:?}"
    );
    output.textures_delta.clear();
}

#[test]
fn narrow_header_moves_secondary_actions_into_more_menu() {
    let mut app = app();
    app.config.sidebar_expanded = false;
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(
            pos2(0.0, 0.0),
            MIN_WINDOW_SIZE_COLLAPSED,
        )),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    assert!(
        texts.iter().any(|text| text == "更多"),
        "窄窗口应显示更多菜单：{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text == "开始识别"),
        "窄窗口仍应保留主操作：{texts:?}"
    );
    for hidden in [
        "检查更新",
        "说明",
        "界面主题",
        "置顶",
        "地图浮窗",
        "精简界面",
    ] {
        assert!(
            !texts.iter().any(|text| text == hidden),
            "窄窗口不应在顶栏直接显示 {hidden}：{texts:?}"
        );
    }
    output.textures_delta.clear();
}

#[test]
fn entering_compact_mode_shows_exit_hint() {
    let mut app = app();
    assert!(app.compact_hint_until.is_none());
    app.enter_compact_mode();
    assert!(app.compact);
    assert!(
        app.compact_hint_until.is_some(),
        "进入精简模式应显示按 Esc 退出的提示"
    );
}

#[test]
fn map_window_mode_gives_each_task_its_own_popout() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
        ..Default::default()
    };
    assert!(!app.config.map_window_enabled, "地图浮窗默认关闭");

    app.set_map_window(true);
    assert!(app.config.map_window_enabled);
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    assert!(
        texts.iter().any(|text| text == "地图已在独立浮窗中显示"),
        "开启浮窗后主窗口应显示占位说明：{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text == "抓鬼浮窗：开"),
        "占位区应为每个任务提供独立开关：{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text == "鬼王浮窗：开"),
        "鬼王也应单独一个浮窗：{texts:?}"
    );
    output.textures_delta.clear();

    let zhuagui = app
        .config
        .map_window_layouts
        .get("抓鬼")
        .copied()
        .expect("抓鬼浮窗应有默认位置");
    let ghost = app
        .config
        .map_window_layouts
        .get("鬼王")
        .copied()
        .expect("鬼王浮窗应有默认位置");
    assert!(
        (zhuagui.x - ghost.x).abs() > 1.0 || (zhuagui.y - ghost.y).abs() > 1.0,
        "两个任务的浮窗初始位置应错开，避免完全叠在一起"
    );

    // 单个任务浮窗可以独立关闭，不影响另一个任务。
    app.set_map_window_visible("抓鬼", false);
    assert!(!app.map_window_open("抓鬼"));
    assert!(app.map_window_open("鬼王"));
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    assert!(texts.iter().any(|text| text == "抓鬼浮窗：关"));
    assert!(texts.iter().any(|text| text == "鬼王浮窗：开"));
    output.textures_delta.clear();

    // 取消勾选鬼王后它的浮窗消失，但布局仍保留在配置里。
    app.config.ghost_king_enabled = false;
    assert!(!app.map_window_open("鬼王"));
    app.set_map_window(false);
    assert!(!app.config.map_window_enabled);
    assert!(
        app.config.map_window_layouts.contains_key("抓鬼")
            && app.config.map_window_layouts.contains_key("鬼王"),
        "关闭浮窗模式不应丢掉各窗口的位置和大小"
    );
}

#[test]
fn map_window_resize_hit_test_prefers_corners_then_edges() {
    use egui::ResizeDirection;
    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(400.0, 300.0));
    let border = 6.0;
    assert_eq!(
        map_window::resize_direction(rect, rect.center(), border),
        None,
        "窗口中间拖动是移动，不是缩放"
    );
    assert_eq!(
        map_window::resize_direction(rect, pos2(rect.left() + 2.0, rect.center().y), border),
        Some(ResizeDirection::West)
    );
    assert_eq!(
        map_window::resize_direction(rect, pos2(rect.right() - 2.0, rect.center().y), border),
        Some(ResizeDirection::East)
    );
    assert_eq!(
        map_window::resize_direction(rect, pos2(rect.center().x, rect.top() + 2.0), border),
        Some(ResizeDirection::North)
    );
    assert_eq!(
        map_window::resize_direction(rect, pos2(rect.center().x, rect.bottom() - 2.0), border),
        Some(ResizeDirection::South)
    );
    assert_eq!(
        map_window::resize_direction(rect, pos2(rect.left() + 2.0, rect.top() + 2.0), border),
        Some(ResizeDirection::NorthWest)
    );
    assert_eq!(
        map_window::resize_direction(rect, pos2(rect.right() - 2.0, rect.top() + 2.0), border),
        Some(ResizeDirection::NorthEast)
    );
    assert_eq!(
        map_window::resize_direction(rect, pos2(rect.left() + 2.0, rect.bottom() - 2.0), border),
        Some(ResizeDirection::SouthWest)
    );
    assert_eq!(
        map_window::resize_direction(rect, pos2(rect.right() - 2.0, rect.bottom() - 2.0), border),
        Some(ResizeDirection::SouthEast)
    );
}

#[test]
fn map_window_opacity_stays_in_usable_range() {
    let mut app = app();
    app.config.map_window_opacity = 0.0;
    assert_eq!(
        app.map_window_alpha(),
        (map_window::MAP_WINDOW_MIN_OPACITY * 255.0).round() as u8,
        "透明度不能低到完全看不见"
    );
    app.config.map_window_opacity = 1.0;
    assert_eq!(app.map_window_alpha(), 255);
}

#[test]
fn vote_uses_median_for_same_map() {
    let mut votes = VoteBuffer::default();
    assert_eq!(
        votes.vote("长寿村", 100, 50),
        ("长寿村".to_string(), 100, 50)
    );
    votes.vote("长寿村", 102, 52);
    let (map, x, y) = votes.vote("长寿村", 101, 50);
    assert_eq!(map, "长寿村");
    assert_eq!((x, y), (101, 50), "同地图应取坐标中位数");
}

#[test]
fn vote_switches_map_after_two_frames() {
    let mut votes = VoteBuffer::default();
    for _ in 0..3 {
        votes.vote("长寿村", 100, 50);
    }
    let (map, _, _) = votes.vote("天宫", 200, 60);
    assert_eq!(map, "长寿村", "单帧新地图不应立刻切换");
    let (map, _, _) = votes.vote("天宫", 201, 61);
    assert_eq!(map, "天宫", "连续两帧新地图才切换");
}

#[test]
fn compact_mode_hides_chrome_and_esc_restores_it() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
        ..Default::default()
    };

    app.compact = true;
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    for hidden in [
        "开始识别",
        "精简界面",
        "置顶",
        "界面主题",
        "说明",
        "梦幻西游",
    ] {
        assert!(
            !texts.iter().any(|text| text == hidden),
            "精简模式不应显示 {hidden}：{texts:?}"
        );
    }
    assert!(
        !texts.iter().any(|text| text.contains("检查更新")),
        "精简模式不应显示更新按钮：{texts:?}"
    );
    output.textures_delta.clear();

    // Esc 回到默认界面，顶部按钮和侧栏重新出现。
    let mut output = ctx.run_ui(escape_input(), |ui| app.ui(ui, &mut frame));
    assert!(!app.compact, "按 Esc 应退出精简模式");
    let texts = collect_texts(&output.shapes);
    assert!(
        texts.iter().any(|text| text == "开始识别"),
        "退出精简模式后应重新出现开始识别按钮：{texts:?}"
    );
    output.textures_delta.clear();
}

fn result(generation: u64) -> OcrEvent {
    OcrEvent::Remote {
        generation,
        result: Ok(OcrText {
            text: "抓鬼 长寿村 12,34 鬼王 天宫 56,78".into(),
            elapsed_secs: 0.25,
        }),
    }
}

#[test]
fn switching_windows_discards_inflight_results_and_previous_coordinates() {
    let mut app = app();
    app.set_window(Some(1));
    app.apply_ocr(&OcrText {
        text: "抓鬼 长寿村 12,34".into(),
        elapsed_secs: 0.1,
    });
    assert!(app.tasks["抓鬼"].location.is_some());
    app.busy = true;
    let old_generation = app.generation;
    app.set_window(Some(2));
    app.sender.send(result(old_generation)).unwrap();
    app.poll_events();
    assert!(!app.busy);
    assert!(app.tasks.values().all(|state| state.location.is_none()));
}

#[test]
fn pausing_discards_inflight_results() {
    let mut app = app();
    app.running = true;
    app.busy = true;
    let generation = app.generation;
    app.toggle_running();
    app.sender.send(result(generation)).unwrap();
    app.poll_events();
    assert!(!app.running);
    assert!(!app.busy);
    assert!(app.tasks["抓鬼"].location.is_none());
}

#[test]
fn remote_test_feedback_is_available_with_simple_logging() {
    let mut app = app();
    app.config.log_level = "detailed".into();
    app.log_path =
        std::env::temp_dir().join(format!("rust-mhxy-log-test-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&app.log_path);
    app.testing_remote = true;
    app.sender
        .send(OcrEvent::Test {
            result: Err("连接超时".into()),
        })
        .unwrap();
    app.poll_events();
    assert!(!app.testing_remote);
    assert_eq!(app.remote_feedback, Some(Err("连接超时".into())));
    let text = std::fs::read_to_string(&app.log_path).unwrap_or_default();
    assert!(text.contains("连接超时"), "日志应直接写入磁盘：{text:?}");
    let _ = std::fs::remove_file(&app.log_path);
}

#[test]
fn region_window_renders_all_preprocess_modes() {
    let mut app = app();
    app.show_region_window = true;
    app.config.preprocess_mode = "color".into();
    app.config.preprocess_color = [240, 210, 120];
    app.update_preview(image::RgbImage::from_pixel(
        320,
        240,
        image::Rgb([24, 24, 24]),
    ));
    let ctx = app.ctx.clone();
    for mode in [
        "off",
        "gray",
        "binary",
        "color",
        "color,binary",
        "gray,binary",
        "binary,invert",
    ] {
        app.config.preprocess_mode = mode.into();
        app.refresh_preprocess_preview();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
                ..Default::default()
            },
            |ui| app.region_window_ui(ui),
        );
        assert!(!output.shapes.is_empty(), "预处理模式 {mode} 渲染失败");
        output.textures_delta.clear();
    }
}

#[test]
fn update_events_move_through_phases() {
    let mut app = app();
    let info = UpdateInfo {
        version: "v9.9.9".into(),
        notes: "测试".into(),
        url: "https://example.com/a.zip".into(),
        sha256: "abc".into(),
        size: 100,
    };
    app.update_tx
        .send(UpdateEvent::Checked(Ok(Some(info))))
        .unwrap();
    app.poll_updates();
    assert_eq!(app.update_phase, UpdatePhase::Available);
    assert_eq!(
        app.update_info.as_ref().map(|info| info.version.as_str()),
        Some("v9.9.9")
    );

    app.update_tx
        .send(UpdateEvent::Downloaded(Ok(PathBuf::from("/tmp/a.zip"))))
        .unwrap();
    app.poll_updates();
    assert_eq!(app.update_phase, UpdatePhase::Downloaded);
    assert!(app.update_zip.is_some());

    app.update_tx
        .send(UpdateEvent::Checked(Err("网络错误".into())))
        .unwrap();
    app.poll_updates();
    assert_eq!(app.update_phase, UpdatePhase::Failed);
    assert_eq!(app.update_error.as_deref(), Some("网络错误"));
}

#[test]
fn update_section_renders_every_phase() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    app.update_info = Some(UpdateInfo {
        version: "v9.9.9".into(),
        notes: "测试说明".into(),
        url: "https://example.com/a.zip".into(),
        sha256: "abc".into(),
        size: 1024,
    });
    app.update_message = Some("已是最新版本".into());
    app.update_message_at = Some(Instant::now());
    app.update_error = Some("测试错误".into());
    app.update_progress.store(512, Ordering::Relaxed);
    for phase in [
        UpdatePhase::Idle,
        UpdatePhase::Checking,
        UpdatePhase::Available,
        UpdatePhase::Downloading,
        UpdatePhase::Downloaded,
        UpdatePhase::Failed,
    ] {
        app.update_phase = phase;
        // 同时渲染顶部按钮和更新说明弹窗。
        app.show_update_dialog = true;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
                ..Default::default()
            },
            |ui| app.ui(ui, &mut frame),
        );
        assert!(!output.shapes.is_empty(), "更新阶段 {phase:?} 渲染失败");
        app.show_update_dialog = false;
        output.textures_delta.clear();
    }
}

#[test]
fn panel_test_result_is_formatted_for_region_window() {
    let mut app = app();
    app.ocr_test_running = true;
    app.sender
        .send(OcrEvent::PanelTest {
            result: Ok(OcrText {
                text: "抓鬼 长寿村 12,34".into(),
                elapsed_secs: 0.2,
            }),
        })
        .unwrap();
    app.poll_events();
    assert!(!app.ocr_test_running, "测试结束后应复位状态");
    let text = app
        .ocr_test_result
        .as_ref()
        .expect("应保存测试结果")
        .as_ref()
        .expect("应识别成功");
    assert!(text.contains("耗时 0.20 秒"));
    assert!(text.contains("抓鬼"));
    assert!(text.contains("长寿村"));
    assert!(text.contains("(12, 34)"));

    app.ocr_test_running = true;
    app.sender
        .send(OcrEvent::PanelTest {
            result: Err("模型加载失败".into()),
        })
        .unwrap();
    app.poll_events();
    assert_eq!(app.ocr_test_result, Some(Err("模型加载失败".to_string())));
}

#[test]
fn region_window_uses_left_right_layout() {
    fn mesh_bounds(shape: &egui::Shape, texture: egui::TextureId) -> Option<Rect> {
        match shape {
            egui::Shape::Mesh(mesh) if mesh.texture_id == texture => Some(mesh.calc_bounds()),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| mesh_bounds(shape, texture)),
            _ => None,
        }
    }
    let mut app = app();
    app.show_region_window = true;
    app.config.preprocess_mode = "binary".into();
    app.config.extra_region_enabled = true;
    app.update_preview(image::RgbImage::from_pixel(
        640,
        360,
        image::Rgb([20, 20, 20]),
    ));
    let ctx = app.ctx.clone();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 820.0))),
            ..Default::default()
        },
        |ui| app.region_window_ui(ui),
    );
    let preview_id = app.preview_texture.as_ref().expect("预览纹理").id();
    let preprocess_id = app.preprocess_preview.as_ref().expect("预处理预览").id();
    let extra_preview_id = app
        .preprocess_preview_extra
        .as_ref()
        .expect("备用区域预处理预览")
        .id();
    let preview = output
        .shapes
        .iter()
        .find_map(|shape| mesh_bounds(&shape.shape, preview_id))
        .expect("识别区域预览未绘制");
    let preprocess = output
        .shapes
        .iter()
        .find_map(|shape| mesh_bounds(&shape.shape, preprocess_id))
        .expect("预处理预览未绘制");
    assert!(preview.min.x >= 300.0, "识别区域预览应在右侧：{preview:?}");
    assert!(
        preprocess.min.x >= 300.0,
        "预处理预览应在右侧：{preprocess:?}"
    );
    assert!(
        preprocess.min.y > preview.max.y,
        "预处理预览应在识别区域预览下方：preview={preview:?} preprocess={preprocess:?}"
    );
    let extra_preprocess = output
        .shapes
        .iter()
        .find_map(|shape| mesh_bounds(&shape.shape, extra_preview_id))
        .expect("备用区域预处理预览未绘制");
    assert!(
        preprocess.min.x < extra_preprocess.min.x,
        "主区域在左、备用区域在右：main={preprocess:?} extra={extra_preprocess:?}"
    );
    assert!(
        preprocess.min.y < extra_preprocess.max.y && extra_preprocess.min.y < preprocess.max.y,
        "两张预处理预览应左右排列：main={preprocess:?} extra={extra_preprocess:?}"
    );

    // 文字位置：截图范围、门控设置在上排左侧；识别预处理在分隔线下方。
    fn text_pos(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<egui::Pos2> {
        fn walk(shape: &egui::Shape, needle: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) => text.galley.text().contains(needle).then_some(text.pos),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| walk(shape, needle)),
                _ => None,
            }
        }
        shapes.iter().find_map(|shape| walk(&shape.shape, needle))
    }
    let scope = text_pos(&output.shapes, "截图范围").expect("缺少截图范围标题");
    let extra_region = text_pos(&output.shapes, "备用区域").expect("缺少备用区域标题");
    let gate = text_pos(&output.shapes, "图色门控设置").expect("缺少门控标题");
    let preprocess_title = text_pos(&output.shapes, "识别预处理").expect("缺少预处理标题");
    let preview_title = text_pos(&output.shapes, "识别区域预览").expect("缺少预览标题");
    assert!(
        scope.y < extra_region.y && extra_region.y < gate.y,
        "顺序应为 截图范围 → 备用区域 → 门控设置：{scope:?} {extra_region:?} {gate:?}"
    );
    let main_region = text_pos(&output.shapes, "主区域").expect("缺少主区域标题");
    let enable_extra = text_pos(&output.shapes, "启用备用区域").expect("缺少启用备用区域");
    let main_preprocess = text_pos(&output.shapes, "主区域预处理").expect("缺少主区域预处理");
    let extra_preprocess = text_pos(&output.shapes, "备用区域预处理").expect("缺少备用区域预处理");
    assert!(
        main_region.y < extra_region.y && extra_region.y < enable_extra.y,
        "截图范围内顺序应为 主区域 → 备用区域 → 启用备用区域"
    );
    assert!(
            main_preprocess.y > gate.y && main_preprocess.y < extra_preprocess.y,
            "预处理应为 主区域在上、备用区域在下，且在门控设置之后：{main_preprocess:?} {extra_preprocess:?}"
        );
    assert!(
        preprocess_title.y > gate.y,
        "识别预处理应在分隔线下方、门控设置之后"
    );
    assert!(
        preview_title.x > scope.x + 120.0,
        "识别区域预览标题应在右侧列"
    );
    output.textures_delta.clear();
}

#[test]
fn preprocess_settings_are_independent_per_region() {
    let mut app = app();
    app.config.preprocess_mode = "binary".into();
    app.config.extra_preprocess_mode = "color".into();
    app.config.extra_preprocess_color = [1, 2, 3];
    app.config.extra_region_enabled = true;
    app.show_region_window = true;
    app.update_preview(image::RgbImage::from_pixel(
        200,
        100,
        image::Rgb([30, 30, 30]),
    ));
    assert_eq!(app.preprocess_pipeline_for(false).config_value(), "binary");
    assert_eq!(app.preprocess_pipeline_for(true).config_value(), "color");
    assert!(app.preprocess_preview.is_some(), "主区域预览应生成");
    assert!(app.preprocess_preview_extra.is_some(), "备用区域预览应生成");

    // 修改主区域设置不影响备用区域。
    app.config.preprocess_mode = "gray".into();
    app.refresh_preprocess_preview();
    assert_eq!(app.preprocess_pipeline_for(true).config_value(), "color");
}

#[test]
fn preprocess_preview_tracks_settings() {
    let mut app = app();
    app.show_region_window = true;
    app.config.preprocess_mode = "binary".into();
    app.update_preview(image::RgbImage::from_pixel(
        60,
        40,
        image::Rgb([10, 10, 10]),
    ));
    assert!(
        app.preprocess_preview.is_some(),
        "取点窗口打开时应生成预处理预览"
    );
    let first = app.preprocess_preview.as_ref().map(|texture| texture.id());
    app.config.preprocess_mode = "gray".into();
    app.refresh_preprocess_preview();
    assert_eq!(
        app.preprocess_preview.as_ref().map(|texture| texture.id()),
        first,
        "切换模式应复用同一纹理"
    );

    app.show_region_window = false;
    app.release_preview();
    assert!(app.preprocess_preview.is_none(), "关闭取点窗口后释放预览");
}

#[test]
fn preview_updates_reuse_the_existing_texture() {
    let mut app = app();
    app.show_region_window = true;
    app.update_preview(image::RgbImage::from_pixel(100, 100, image::Rgb([0, 0, 0])));
    let texture_id = app.preview_texture.as_ref().unwrap().id();
    app.update_preview(image::RgbImage::from_pixel(
        120,
        100,
        image::Rgb([255, 0, 0]),
    ));
    assert_eq!(app.preview_texture.as_ref().unwrap().id(), texture_id);
    assert_eq!(
        sample_image(app.preview_image.as_ref().unwrap(), 119, 99),
        Some([255, 0, 0])
    );
}

#[test]
fn preview_is_released_when_region_window_is_closed() {
    let mut app = app();
    app.show_region_window = true;
    app.update_preview(image::RgbImage::from_pixel(64, 64, image::Rgb([1, 2, 3])));
    assert!(app.preview_image.is_some());
    assert!(app.preview_texture.is_some());
    assert_eq!(app.client_size, Some((64, 64)));

    app.show_region_window = false;
    app.release_preview();
    assert!(app.preview_image.is_none());
    assert!(app.preview_texture.is_none());
    assert_eq!(app.client_size, Some((64, 64)), "客户区尺寸仍用于图色取点");

    app.update_preview(image::RgbImage::from_pixel(80, 60, image::Rgb([4, 5, 6])));
    assert!(app.preview_image.is_none());
    assert!(app.preview_texture.is_none());
    assert_eq!(app.client_size, Some((80, 60)));
}

#[test]
fn failed_recognition_keeps_previous_map_and_bounds() {
    fn map_bounds(shape: &egui::Shape, texture: egui::TextureId) -> Option<Rect> {
        match shape {
            egui::Shape::Mesh(mesh) if mesh.texture_id == texture => Some(mesh.calc_bounds()),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| map_bounds(shape, texture)),
            _ => None,
        }
    }
    let mut app = app();
    let ctx = app.ctx.clone();
    let success = OcrText {
        text: "抓鬼 长寿村 12,34".into(),
        elapsed_secs: 0.1,
    };
    for width in [700.0, 320.0] {
        let mut expected = None;
        for recognized in [true, false, true, false] {
            if recognized {
                app.apply_ocr(&success);
            } else {
                app.apply_ocr(&OcrText {
                    text: String::new(),
                    elapsed_secs: 0.1,
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 400.0))),
                    ..Default::default()
                },
                |ui| app.task_panel(ui, "抓鬼"),
            );
            let texture = app.map_textures.values().next().unwrap().id();
            let bounds = output
                .shapes
                .iter()
                .find_map(|shape| map_bounds(&shape.shape, texture))
                .unwrap();
            assert!(
                app.tasks["抓鬼"].location.is_some(),
                "未识别到任务时应保留上次坐标"
            );
            if let Some(expected) = expected {
                assert_eq!(bounds, expected, "宽度：{width}，本轮识别：{recognized}");
            }
            expected = Some(bounds);
            output.textures_delta.clear();
        }
    }
}

#[test]
fn renders_in_light_theme() {
    let ctx = egui::Context::default();
    theme::install(&ctx, true);
    let mut app = MhxyApp::from_config(
        Config {
            theme: "light".into(),
            ..Default::default()
        },
        PathBuf::from("/unused/config.json"),
        ctx.clone(),
    );
    let mut frame = eframe::Frame::_new_kittest();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
            ..Default::default()
        },
        |ui| app.ui(ui, &mut frame),
    );
    assert!(!output.shapes.is_empty(), "亮色主题渲染失败");
    output.textures_delta.clear();
    // 恢复默认，避免影响并行运行的其他测试。
    theme::set_light(&ctx, false);
}

#[test]
fn profiles_apply_save_and_delete() {
    let mut app = app();
    app.config.preprocess_mode = "binary".into();
    app.config.color_gate_enabled = true;
    app.save_active_profile();

    app.config.preprocess_mode = "off".into();
    app.config.color_gate_enabled = false;
    let name = app.config.active_profile.clone();
    app.apply_profile(&name);
    assert_eq!(app.config.preprocess_mode, "binary");
    assert!(app.config.color_gate_enabled);

    app.profile_name_edit = "测试方案".into();
    app.create_profile();
    assert_eq!(app.config.active_profile, "测试方案");
    assert!(app.config.profiles.iter().any(|p| p.name == "测试方案"));

    app.delete_active_profile();
    assert!(!app.config.profiles.iter().any(|p| p.name == "测试方案"));
    assert!(!app.config.profiles.is_empty());
}

#[test]
fn sidebar_sections_can_collapse() {
    let mut app = app();
    app.config.sidebar_folded =
        "window,trigger,ocr,task,log,advanced,region,profiles,display,meditation,flight_flags"
            .into();
    assert!(!app.sidebar_section_open("window"));
    assert!(!app.sidebar_section_open("log"));
    assert!(!app.sidebar_section_open("advanced"));
    assert!(!app.sidebar_section_open("region"));
    assert!(!app.sidebar_section_open("profiles"));
    assert!(!app.sidebar_section_open("display"));
    assert!(!app.sidebar_section_open("meditation"));
    assert!(!app.sidebar_section_open("flight_flags"));
    assert!(app.sidebar_section_open("unknown"));
    app.set_sidebar_section_open("window", true);
    assert!(app.sidebar_section_open("window"));
    assert!(app.config.sidebar_folded.contains("trigger"));
    app.set_sidebar_section_open("trigger", true);
    assert!(!app.config.sidebar_folded.contains("trigger"));

    // 全折叠时侧栏仍能正常渲染。
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
            ..Default::default()
        },
        |ui| app.ui(ui, &mut frame),
    );
    assert!(!output.shapes.is_empty());
    output.textures_delta.clear();
}

#[test]
fn region_window_stays_on_screen_and_prefers_the_free_side() {
    let size = vec2(900.0, 700.0);
    let monitor = vec2(1920.0, 1080.0);

    // 主窗口靠左、右侧放得下：贴主窗口右缘。
    let left_half = Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 800.0));
    assert_eq!(
        place_region_window(left_half, monitor, size),
        pos2(908.0, 0.0)
    );

    // 主窗口靠右、右侧放不下但左侧放得下：改放主窗口左侧，两边都不重叠。
    let right_half = Rect::from_min_size(pos2(1000.0, 0.0), vec2(900.0, 800.0));
    assert_eq!(
        place_region_window(right_half, monitor, size),
        pos2(92.0, 0.0)
    );

    // 两侧都放不下（默认窗口尺寸 + 1080p 正是这种情况）：贴屏幕右缘，整窗仍在屏内。
    let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0));
    assert_eq!(place_region_window(wide, monitor, size), pos2(1020.0, 0.0));

    // 主窗口贴近屏幕下沿时整体上移，底边不越出屏幕（算上了标题栏高度）。
    let low = Rect::from_min_size(pos2(0.0, 900.0), vec2(1120.0, 180.0));
    assert_eq!(place_region_window(low, monitor, size), pos2(1020.0, 348.0));
}

#[test]
fn generic_task_panel_follows_config() {
    let mut app = app();
    assert!(app.task_active("抓鬼"), "抓鬼默认开启");
    assert!(!app.task_active("通用"), "默认不显示通用任务面板");
    app.config.generic_enabled = true;
    assert!(app.task_active("通用"), "勾选后应显示通用任务面板");
    assert_eq!(TASK_ORDER, ["抓鬼", "鬼王", "通用"]);

    // 通用任务与抓鬼互斥：切到通用后抓鬼不再显示。
    app.config.zhuagui_enabled = false;
    assert!(!app.task_active("抓鬼"));
    assert!(app.task_active("通用"));
}

#[test]
fn long_window_titles_do_not_push_sidebar_controls_out_of_the_card() {
    /// 在绘制出来的图元里按文本找控件的实际位置。
    fn text_bounds(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<Rect> {
        fn walk(shape: &egui::Shape, needle: &str) -> Option<Rect> {
            match shape {
                egui::Shape::Text(text) if text.galley.text().contains(needle) => {
                    Some(text.visual_bounding_rect())
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| walk(shape, needle)),
                _ => None,
            }
        }
        shapes
            .iter()
            .find_map(|clipped| walk(&clipped.shape, needle))
    }

    let mut app = app();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    app.config.sidebar_expanded = true;
    app.selected_handle = Some(7);
    app.windows = vec![WindowInfo {
        handle: 7,
        title: "梦幻西游 ONLINE - (山东1区[东海崂山]) - 守^一座空城[41] - 再补一段很长的标题"
            .into(),
    }];

    // 侧栏展开时的最小窗口：此时侧栏取到上限 400，最容易挤掉右侧控件。
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(760.0, 540.0))),
            ..Default::default()
        },
        |ui| app.ui(ui, &mut frame),
    );
    output.textures_delta.clear();

    // ComboBox::width() 只是最小宽度，长标题曾把「刷新」整个挤出卡片。
    // 注意：被挤出侧栏的控件会被 egui 按 clip_rect 剔除，所以「找不到」也算失败。
    for control in ["刷新", "设置识别区域"] {
        let bounds = text_bounds(&output.shapes, control)
            .unwrap_or_else(|| panic!("「{control}」没有绘制在侧栏可见区域内"));
        assert!(
            bounds.max.x <= 400.0,
            "「{control}」超出了侧栏右缘：{bounds:?}"
        );
    }
}

#[test]
fn window_controls_send_viewport_commands() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
        ..Default::default()
    };

    app.config.on_top = false;
    app.set_on_top(true, &ctx);
    assert!(app.config.on_top);
    assert!(app.config_dirty_since.is_some(), "切换置顶后应触发配置保存");
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    assert!(output.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .contains(&egui::ViewportCommand::WindowLevel(
            egui::WindowLevel::AlwaysOnTop
        )));
    output.textures_delta.clear();

    app.config.on_top = true;
    app.set_on_top(false, &ctx);
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    assert!(output.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .contains(&egui::ViewportCommand::WindowLevel(
            egui::WindowLevel::Normal
        )));
    output.textures_delta.clear();
}

#[test]
fn collapsing_sidebar_lowers_minimum_window_size() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
        ..Default::default()
    };
    let min_size = |output: &egui::FullOutput| {
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .find_map(|command| match command {
                egui::ViewportCommand::MinInnerSize(size) => Some(*size),
                _ => None,
            })
    };

    app.config.sidebar_expanded = true;
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    assert_eq!(min_size(&output), Some(MIN_WINDOW_SIZE_EXPANDED));
    output.textures_delta.clear();

    app.config.sidebar_expanded = false;
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let collapsed = min_size(&output).expect("收起侧栏后应放宽最小尺寸");
    assert_eq!(collapsed, MIN_WINDOW_SIZE_COLLAPSED);
    assert!(collapsed.x < MIN_WINDOW_SIZE_EXPANDED.x);
    output.textures_delta.clear();
}

#[test]
fn renders_local_remote_and_map_layouts_at_supported_sizes() {
    /// 找到地图纹理的绘制区域。
    fn map_bounds(shape: &egui::Shape, texture: egui::TextureId) -> Option<Rect> {
        match shape {
            egui::Shape::Mesh(mesh) if mesh.texture_id == texture => Some(mesh.calc_bounds()),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| map_bounds(shape, texture)),
            _ => None,
        }
    }
    let mut app = app();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    for (size, remote, horizontal, sidebar, with_task) in [
        ([1120.0, 800.0], false, false, true, false),
        ([760.0, 540.0], true, false, true, false),
        ([1120.0, 800.0], false, true, true, true),
        ([760.0, 540.0], false, false, false, true),
    ] {
        app.config.ocr_mode = if remote { "remote" } else { "local" }.into();
        app.config.horizontal_layout = horizontal;
        app.config.sidebar_expanded = sidebar;
        if with_task {
            app.apply_ocr(&OcrText {
                text: "抓鬼 长寿村 12,34 鬼王 天宫 56,78".into(),
                elapsed_secs: 0.1,
            });
        }
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::from(size))),
                ..Default::default()
            },
            |ui| app.ui(ui, &mut frame),
        );
        assert!(!output.shapes.is_empty());
        assert!(output
            .shapes
            .iter()
            .all(|shape| shape.clip_rect.is_finite()));
        if sidebar {
            // 地图必须在侧栏右侧绘制，不能被侧栏区域覆盖。
            for texture in app.map_textures.values().map(|handle| handle.id()) {
                if let Some(bounds) = output
                    .shapes
                    .iter()
                    .find_map(|shape| map_bounds(&shape.shape, texture))
                {
                    assert!(
                        bounds.min.x >= 240.0,
                        "地图绘到了侧栏区域：{bounds:?}（窗口 {size:?}）"
                    );
                }
            }
        }
        output.textures_delta.clear();
    }
    assert_eq!(app.map_textures.len(), 2);
}
