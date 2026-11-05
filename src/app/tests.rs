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
    // 侧栏不再有打坐分组；顶栏只留一个「更多」菜单，打坐和合成旗在其中的工具分组里。
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(
        texts.iter().any(|text| text == "更多"),
        "顶栏应有唯一的更多菜单：{texts:?}"
    );
    assert!(
        !texts.iter().any(|text| text == "工具"),
        "工具分组只应在更多菜单里：{texts:?}"
    );
    assert!(
        !texts.iter().any(|text| text == "启用打坐计时"),
        "打坐设置不应再出现在侧栏：{texts:?}"
    );

    // 工具菜单里的两个入口。
    let mut output = ctx.run_ui(input(), |ui| app.tools_menu_ui(ui, &ctx));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    for expected in ["打坐计时", "合成旗编辑器"] {
        assert!(
            texts.iter().any(|text| text == expected),
            "工具菜单缺少「{expected}」：{texts:?}"
        );
    }

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
        "显示与布局",
        "设置识别区域",
    ] {
        assert!(
            texts.iter().any(|text| text == expected),
            "基础侧栏缺少「{expected}」：{texts:?}"
        );
    }
    // 主操作固定在顶栏，次级入口全部收进「更多」，侧栏不重复。
    for expected in ["开始识别", "更多"] {
        assert!(
            texts.iter().any(|text| text == expected),
            "顶栏缺少「{expected}」：{texts:?}"
        );
    }
    // 高级设置、工具菜单内容默认收起，低频项不应出现在首屏。
    for hidden in [
        "远端 PP-OCR",
        "服务地址",
        "访问令牌（可选）",
        "识别方案",
        "打坐计时",
        "合成旗标注",
        "日志",
        "地图浮窗（无边框）",
    ] {
        assert!(
            !texts.iter().any(|text| text == hidden),
            "默认收起时不应显示「{hidden}」：{texts:?}"
        );
    }
    // 底栏始终给出当前识别方式、触发摘要和更新入口。
    assert!(texts.iter().any(|text| text == "本地 PP-OCRv5"));
    assert!(texts.iter().any(|text| text.contains("每 5 秒识别")));

    let find = |needle: &str| {
        positions
            .iter()
            .find(|(text, _)| text == needle)
            .map(|(_, pos)| *pos)
            .unwrap_or_else(|| panic!("界面缺少「{needle}」：{positions:?}"))
    };
    let window = find("游戏窗口");
    let trigger = find("识别模式");
    let task = find("任务类型");
    let starts: Vec<egui::Pos2> = positions
        .iter()
        .filter(|(text, _)| text == "开始识别")
        .map(|(_, pos)| *pos)
        .collect();
    assert_eq!(starts.len(), 1, "开始按钮只应出现在顶栏一次：{starts:?}");
    let start = starts[0];
    assert!(
        window.y < trigger.y && trigger.y < task.y,
        "侧栏顺序应为 游戏窗口 → 识别模式 → 任务类型：window={window:?} trigger={trigger:?} task={task:?}"
    );
    assert!(
        start.y < window.y && start.x > 420.0,
        "开始按钮应在顶栏右侧，而不是侧栏底部：start={start:?} window={window:?}"
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
    assert!(
        texts.iter().any(|text| text == "实时模式"),
        "自动模式应提供实时模式开关：{texts:?}"
    );
    assert!(!texts.iter().any(|text| text == "快捷键"));

    // 实时模式开启后开关仍然可见（否则关不掉）。
    app.config.realtime = true;
    let mut output = ctx.run_ui(input(), |ui| app.basic_controls(ui));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(
        texts.iter().any(|text| text == "实时模式"),
        "实时模式开启后开关仍需可见：{texts:?}"
    );

    app.config.realtime = false;
    app.config.trigger_mode = "manual".into();
    let mut output = ctx.run_ui(input(), |ui| app.basic_controls(ui));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(
        !texts.iter().any(|text| text == "实时模式"),
        "手动模式不需要实时模式开关：{texts:?}"
    );
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
fn header_start_button_follows_trigger_mode_and_running() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
        ..Default::default()
    };

    app.config.trigger_mode = "manual".into();
    app.running = false;
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(texts.iter().any(|text| text == "开始监听"));
    assert!(texts
        .iter()
        .any(|text| text.contains("手动识别 · 快捷键 F8")));

    app.running = true;
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(texts.iter().any(|text| text == "暂停监听"));
    assert!(texts.iter().any(|text| text.contains("手动识别 · 按 F8")));

    app.config.trigger_mode = "auto".into();
    app.config.interval_seconds = 5;
    app.running = false;
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(texts.iter().any(|text| text == "开始识别"));
    assert!(texts.iter().any(|text| text == "每 5 秒识别"));
}

#[test]
fn advanced_settings_hold_low_frequency_controls() {
    let mut app = app();
    app.config.ocr_mode = "remote".into();
    app.config.api_url = "http://127.0.0.1:8008".into();
    app.config.color_gate_enabled = true;
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
fn header_keeps_title_more_and_start_only() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    let positions = text_positions(&output.shapes);
    let find = |needle: &str| {
        positions
            .iter()
            .find(|(text, _)| text == needle)
            .map(|(_, pos)| *pos)
            .unwrap_or_else(|| panic!("顶栏缺少「{needle}」：{positions:?}"))
    };
    assert!(
        texts.iter().any(|text| text == "梦幻西游"),
        "顶栏应保留应用标题：{texts:?}"
    );
    let more = find("更多");
    let start = find("开始识别");
    assert!(
        more.x < start.x,
        "顶栏右侧应为「更多」+「开始识别」：more={more:?} start={start:?}"
    );
    // 原来的分类按钮和平铺按钮都不再直接占位。
    for hidden in [
        "显示方式",
        "显示",
        "工具",
        "设置",
        "帮助",
        "检查更新",
        "说明",
        "界面主题",
        "置顶",
        "精简界面",
        "地图浮窗",
        "高级设置",
        "打坐",
    ] {
        assert!(
            !texts.iter().any(|text| text == hidden),
            "顶栏不应再直接显示「{hidden}」：{texts:?}"
        );
    }
    output.textures_delta.clear();

    // 「更多」菜单里的分组和条目仍然齐全。
    let mut output = ctx.run_ui(input(), |ui| app.more_menu_ui(ui, &ctx));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    for expected in [
        "显示",
        "工具",
        "帮助",
        "设置",
        "普通界面",
        "精简界面",
        "地图浮窗",
        "界面主题",
        "窗口置顶",
        "打坐计时",
        "合成旗编辑器",
        "说明",
        "检查更新",
        "高级设置…",
    ] {
        assert!(
            texts.iter().any(|text| text == expected),
            "更多菜单缺少「{expected}」：{texts:?}"
        );
    }

    // 地图浮窗的参数已搬进独立窗口，菜单里只留入口。
    app.config.map_window_enabled = true;
    let mut output = ctx.run_ui(input(), |ui| app.more_menu_ui(ui, &ctx));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(
        texts.iter().any(|text| text == "浮窗设置…"),
        "地图浮窗开启后菜单应提供「浮窗设置…」入口：{texts:?}"
    );
    for absent in ["浮窗不透明度", "抓鬼宽度", "重置浮窗位置与大小"] {
        assert!(
            !texts.iter().any(|text| text == absent),
            "「{absent}」已经搬到地图浮窗设置窗口，菜单里不再重复：{texts:?}"
        );
    }
}

#[test]
fn narrow_header_keeps_start_and_more_inside_bounds() {
    fn text_bounds(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<Rect> {
        fn walk(shape: &egui::Shape, needle: &str) -> Option<Rect> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == needle => {
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
    app.config.sidebar_expanded = false;
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(
            pos2(0.0, 0.0),
            MIN_WINDOW_SIZE_COLLAPSED,
        )),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    for expected in ["开始", "更多"] {
        assert!(
            texts.iter().any(|text| text == expected),
            "窄窗口顶栏缺少「{expected}」：{texts:?}"
        );
    }
    for label in ["开始", "更多"] {
        let bounds = text_bounds(&output.shapes, label)
            .unwrap_or_else(|| panic!("窄窗口顶栏没有绘制「{label}」"));
        assert!(
            bounds.min.x >= 0.0 && bounds.max.x <= MIN_WINDOW_SIZE_COLLAPSED.x,
            "「{label}」超出窄窗口：{bounds:?}"
        );
    }
    output.textures_delta.clear();

    // 更新状态直接写在「更多」按钮上，菜单里仍保留重启入口。
    app.update_info = Some(UpdateInfo {
        version: "v9.9.9".into(),
        notes: "测试".into(),
        url: "https://example.com/a.zip".into(),
        sha256: "abc".into(),
        size: 1024,
    });
    app.update_phase = UpdatePhase::Downloaded;
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    assert!(
        texts.iter().any(|text| text == "更多·重启"),
        "已下载时窄窗口的更多按钮应直接显示重启状态：{texts:?}"
    );
    let badge = text_bounds(&output.shapes, "更多·重启").expect("窄窗口应绘制更新状态");
    assert!(
        badge.min.x >= 0.0 && badge.max.x <= MIN_WINDOW_SIZE_COLLAPSED.x,
        "更新状态不应把窄窗口顶栏挤出边界：{badge:?}"
    );
    output.textures_delta.clear();

    let mut output = ctx.run_ui(input(), |ui| app.help_menu_ui(ui, &ctx));
    let texts = collect_texts(&output.shapes);
    assert!(
        texts.iter().any(|text| text == "重启并更新"),
        "帮助菜单应保留已下载安装按钮：{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text.contains("安装包已就绪")),
        "帮助菜单应提示安装包已就绪：{texts:?}"
    );
    output.textures_delta.clear();
}

#[test]
fn update_status_is_shown_on_more_button() {
    let mut app = app();
    app.update_info = Some(UpdateInfo {
        version: "v9.9.9".into(),
        notes: "测试".into(),
        url: "https://example.com/a.zip".into(),
        sha256: "abc".into(),
        size: 1024,
    });
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
        ..Default::default()
    };

    app.update_phase = UpdatePhase::Available;
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    assert!(
        texts.iter().any(|text| text == "更多 · 更新 v9.9.9"),
        "有更新时更多按钮应显示版本：{texts:?}"
    );
    output.textures_delta.clear();

    app.update_phase = UpdatePhase::Downloading;
    app.update_progress.store(512, Ordering::Relaxed);
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    assert!(
        texts.iter().any(|text| text == "更多 · 下载 50%"),
        "下载中更多按钮应显示进度：{texts:?}"
    );
    output.textures_delta.clear();

    app.update_phase = UpdatePhase::Downloaded;
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texts = collect_texts(&output.shapes);
    assert!(
        texts.iter().any(|text| text == "更多 · 重启更新"),
        "已下载时更多按钮应显示重启入口：{texts:?}"
    );
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
fn display_mode_choices_are_mutually_exclusive() {
    let mut app = app();
    app.apply_map_window_display();
    assert!(app.config.map_window_enabled);
    assert!(!app.compact);

    app.apply_compact_display();
    assert!(app.compact);
    assert!(!app.config.map_window_enabled, "精简界面与地图浮窗互斥");

    app.apply_normal_display();
    assert!(!app.compact);
    assert!(!app.config.map_window_enabled);

    // 从精简直接切到地图浮窗，也要退出精简。
    app.apply_compact_display();
    app.apply_map_window_display();
    assert!(app.config.map_window_enabled);
    assert!(!app.compact);
}

#[test]
fn sidebar_points_to_map_window_settings_in_header() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(420.0, 900.0))),
        ..Default::default()
    };

    let mut output = ctx.run_ui(input(), |ui| app.basic_controls(ui));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(
        !texts.iter().any(|text| text == "地图浮窗（无边框）"),
        "地图浮窗开关只应在顶栏「更多 → 显示」里：{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|text| text.contains("地图浮窗") && text.contains("更多 → 显示")),
        "侧栏应指向新的浮窗入口：{texts:?}"
    );

    app.config.map_window_enabled = true;
    let mut output = ctx.run_ui(input(), |ui| app.basic_controls(ui));
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    for control in [
        "浮窗不透明度",
        "浮窗置顶",
        "锁定浮窗（鼠标穿透）",
        "重置浮窗位置与大小",
    ] {
        assert!(
            !texts.iter().any(|text| text == control),
            "浮窗参数应集中在顶栏显示分组，侧栏不再重复「{control}」：{texts:?}"
        );
    }
    assert!(
        texts
            .iter()
            .any(|text| text.contains("地图浮窗") && text.contains("更多 → 显示")),
        "开启浮窗后侧栏仍应指向同一组参数：{texts:?}"
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
        texts.iter().any(|text| text == "浮窗设置…"),
        "占位区应提供「浮窗设置…」入口：{texts:?}"
    );
    output.textures_delta.clear();

    // 每个任务的开关搬到了独立的「地图浮窗设置」窗口。
    let mut output = ctx.run_ui(input(), |ui| app.map_window_settings_controls(ui));
    let texts = collect_texts(&output.shapes);
    for expected in [
        "抓鬼浮窗",
        "鬼王浮窗",
        "浮窗不透明度",
        "浮窗置顶",
        "锁定浮窗（鼠标穿透）",
        "重置浮窗位置与大小",
    ] {
        assert!(
            texts.iter().any(|text| text == expected),
            "地图浮窗设置窗口缺少「{expected}」：{texts:?}"
        );
    }
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
    let mut output = ctx.run_ui(input(), |ui| app.map_window_settings_controls(ui));
    let texts = collect_texts(&output.shapes);
    assert!(
        texts.iter().any(|text| text == "抓鬼浮窗"),
        "关掉后这个任务的开关仍应留在设置窗口里：{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text == "鬼王浮窗"),
        "另一个任务的开关不应受影响：{texts:?}"
    );
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
fn map_window_resize_keeps_the_map_aspect_ratio() {
    use egui::ResizeDirection;
    let start = Rect::from_min_size(pos2(100.0, 200.0), vec2(400.0, 300.0));
    let start_pointer = pos2(500.0, 500.0);
    let aspect = 400.0 / 300.0;
    let monitor = vec2(1920.0, 1080.0);

    // 拖右边：宽度 +100，高度按 4:3 增至 375，左上角不动。
    let rect = map_window::resize_rect_keeping_aspect(
        ResizeDirection::East,
        start,
        start_pointer,
        aspect,
        start_pointer + vec2(100.0, 0.0),
        monitor,
    );
    assert!((rect.width() - 500.0).abs() < 0.05, "{rect:?}");
    assert!((rect.height() - 375.0).abs() < 0.05, "{rect:?}");
    assert_eq!(rect.min, start.min);

    // 拖左边：右边界固定，宽度 / 高度同比例变化。
    let rect = map_window::resize_rect_keeping_aspect(
        ResizeDirection::West,
        start,
        start_pointer,
        aspect,
        start_pointer - vec2(50.0, 0.0),
        monitor,
    );
    assert!((rect.width() - 450.0).abs() < 0.05, "{rect:?}");
    assert!((rect.height() - 337.5).abs() < 0.05, "{rect:?}");
    assert!((rect.right() - start.right()).abs() < 0.05, "{rect:?}");

    // 拖上边：下边界固定，高度主导宽度。
    let rect = map_window::resize_rect_keeping_aspect(
        ResizeDirection::North,
        start,
        start_pointer,
        aspect,
        start_pointer - vec2(0.0, 60.0),
        monitor,
    );
    assert!((rect.height() - 360.0).abs() < 0.05, "{rect:?}");
    assert!((rect.width() - 480.0).abs() < 0.05, "{rect:?}");
    assert!((rect.bottom() - start.bottom()).abs() < 0.05, "{rect:?}");

    // 角落拖拽也保持地图比例，并按显示器限制最大尺寸。
    let rect = map_window::resize_rect_keeping_aspect(
        ResizeDirection::SouthEast,
        start,
        start_pointer,
        aspect,
        start_pointer + vec2(5000.0, 5000.0),
        monitor,
    );
    assert!(
        (rect.width() / rect.height() - aspect).abs() < 0.001,
        "角落缩放后比例应保持不变：{rect:?}"
    );
    assert!(
        rect.width() <= monitor.x && rect.height() <= monitor.y,
        "{rect:?}"
    );
}

#[test]
fn native_resize_constraints_keep_aspect_and_fixed_anchor() {
    let minimum = [240, 180];
    let maximum = [1600, 1200];
    let rect = borderless::constrain_sizing(2, [10, 20, 610, 420], 1.5, minimum, maximum, true);
    assert_eq!(rect, [10, 20, 610, 420]);

    let rect = borderless::constrain_sizing(2, [10, 20, 810, 420], 1.5, minimum, maximum, true);
    assert_eq!(rect[2] - rect[0], 800);
    assert_eq!(rect[3] - rect[1], 533);
    assert_eq!(rect[0], 10);
    // 拖右边只向右下长，左上角钉死（以前会按中心重排，窗口跟着上下跳）。
    assert_eq!(rect[1], 20);

    // 拖左边时右上角钉死，上下位置同样不能动。
    let rect = borderless::constrain_sizing(1, [410, 20, 810, 420], 1.5, minimum, maximum, true);
    assert_eq!(rect[0], 810 - 400);
    assert_eq!(rect[2], 810);
    assert_eq!(rect[1], 20);
    assert_eq!(rect[3] - rect[1], 267);

    // 拖下边时只向下、向右长，左边钉死。
    let rect = borderless::constrain_sizing(6, [10, 20, 610, 500], 1.5, minimum, maximum, false);
    assert_eq!(rect[0], 10);
    assert_eq!(rect[1], 20);
    assert_eq!(rect[2] - rect[0], 720);
    assert_eq!(rect[3] - rect[1], 480);

    let rect = borderless::constrain_sizing(4, [100, 100, 500, 300], 2.0, minimum, maximum, false);
    assert_eq!(rect, [100, 100, 500, 300]);
    let rect = borderless::constrain_sizing(4, [100, 100, 500, 500], 2.0, minimum, maximum, false);
    assert_eq!(rect[2] - rect[0], 800);
    assert_eq!(rect[3] - rect[1], 400);
    assert_eq!(rect[2], 500);
    assert_eq!(rect[3], 500);
}

#[test]
fn switching_map_does_not_blow_up_the_window() {
    let monitor = vec2(1920.0, 1080.0);
    let current = vec2(560.0, 420.0);

    // 碗子山 170×340（0.5:1）：直接保留宽度会算出 1120 高，比屏幕还高，
    // 下半截跑到屏幕外、边也抓不到。现在只在原窗口盒子里重排。
    let tall = map_window::resize_for_aspect(current, 170.0 / 340.0, monitor);
    assert!(tall.y <= monitor.y, "不能超出显示器：{tall:?}");
    assert!(tall.y <= 480.5, "换高瘦地图时窗口不该被拉高：{tall:?}");

    // 大唐境外 583×108（≈5.4:1）：宽度不变，只是变矮；不能被「最小高度 × 比例」
    // 反过来把宽度抬到 900px 以上。
    let aspect = 583.0 / 108.0;
    let wide = map_window::resize_for_aspect(current, aspect, monitor);
    assert!(
        (wide.x - 560.0).abs() < 0.5,
        "换极宽地图时宽度不该突变：{wide:?}"
    );
    assert!((wide.y - 560.0 / aspect).abs() < 0.5, "{wide:?}");

    // 普通比例（天宫 420×279）保持原宽度，只按比例调高度。
    let normal = map_window::resize_for_aspect(current, 420.0 / 279.0, monitor);
    assert!((normal.x - 560.0).abs() < 0.5, "{normal:?}");
}

#[test]
fn map_width_limits_never_exceed_the_monitor() {
    let monitor = vec2(1920.0, 1080.0);
    // 高瘦地图按宽度开窗时，高度不能超过显示器。
    let tall = map_window::aspect_size(1120.0, 0.5, monitor);
    assert!(tall.y <= monitor.y, "{tall:?}");
    // 极宽地图不因为最小高度被抬宽。
    let wide = map_window::aspect_size(560.0, 583.0 / 108.0, monitor);
    assert!((wide.x - 560.0).abs() < 0.5, "{wide:?}");
}

#[test]
fn resized_map_window_is_pulled_back_on_screen() {
    let monitor = vec2(1920.0, 1080.0);
    let mut layout = crate::config::MapWindowLayout {
        x: 300.0,
        y: 320.0,
        width: 459.0,
        height: 918.0,
    };
    map_window::clamp_layout_on_screen(&mut layout, monitor);
    assert_eq!(layout.x, 300.0);
    assert_eq!(layout.y, 1080.0 - 918.0, "窗口底边不能超出屏幕");
}

#[test]
fn native_resize_min_width_is_not_inflated_by_tall_min_height() {
    // 极宽地图（大唐境外 ≈5.4:1）按 180 的最小高度反推会得到 970px 以上的
    // 最小宽度，拖一下就突然变宽。
    let rect = borderless::constrain_sizing(
        2,
        [0, 0, 500, 93],
        583.0 / 108.0,
        [240, 180],
        [16384, 16384],
        true,
    );
    assert_eq!(rect[2] - rect[0], 500, "极宽地图不该被最小高度顶宽");
}

#[test]
fn map_window_yields_topmost_while_main_window_is_focused() {
    // 配置要置顶，且主窗口不在前台：浮窗压在其他窗口（游戏）上面。
    assert!(map_window::desired_on_top(true, false));
    // 主窗口在前台（正在调设置 / 开着菜单）：浮窗让位，否则会盖住菜单点不到。
    assert!(!map_window::desired_on_top(true, true));
    // 没开置顶就一直不是置顶。
    assert!(!map_window::desired_on_top(false, false));
    assert!(!map_window::desired_on_top(false, true));
}

#[test]
fn map_window_geometry_change_detection_ignores_rounding_noise() {
    let size = Some(vec2(600.0, 450.0));
    let pos = Some(pos2(100.0, 200.0));
    // 完全没变 / 只在亚像素上抖：不需要重绘。
    assert!(map_window::geometry_close((size, pos), (size, pos)));
    assert!(map_window::geometry_close(
        (size, pos),
        (Some(vec2(600.3, 449.8)), Some(pos2(100.2, 199.9)))
    ));
    // 尺寸或位置真的变了：必须补一帧。
    assert!(!map_window::geometry_close(
        (size, pos),
        (Some(vec2(620.0, 465.0)), pos)
    ));
    assert!(!map_window::geometry_close(
        (size, pos),
        (size, Some(pos2(104.0, 200.0)))
    ));
    // 视口信息还没准备好时不应被当成变化。
    assert!(map_window::geometry_close((None, None), (None, None)));
    assert!(!map_window::geometry_close((size, pos), (None, None)));
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
fn map_window_sizes_itself_to_the_map_image() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
        ..Default::default()
    };
    app.apply_ocr(&OcrText {
        text: "抓鬼 长寿村 12,34".into(),
        elapsed_secs: 0.1,
        regions: Vec::new(),
        notes: Vec::new(),
        ..Default::default()
    });
    // 模拟旧版本存下来的固定 560×460 布局：升级后应自动贴合图片大小，
    // 但仍保留原来的位置。
    app.config.map_window_layouts.insert(
        "抓鬼".into(),
        crate::config::MapWindowLayout {
            x: 100.0,
            y: 120.0,
            width: map_window::MAP_WINDOW_SIZE.x,
            height: map_window::MAP_WINDOW_SIZE.y,
        },
    );
    app.set_map_window(true);
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let texture_size = app
        .map_textures
        .values()
        .next()
        .expect("应加载抓鬼地图")
        .size_vec2();
    let layout = app
        .config
        .map_window_layouts
        .get("抓鬼")
        .copied()
        .expect("抓鬼浮窗应有布局");
    assert!(
        (layout.width - texture_size.x).abs() < 0.5 && (layout.height - texture_size.y).abs() < 0.5,
        "浮窗应贴合地图图片：layout={layout:?} texture={texture_size:?}"
    );
    assert_eq!((layout.x, layout.y), (100.0, 120.0), "贴合图片时保留原位置");
    output.textures_delta.clear();
}

#[test]
fn map_window_builder_geometry_stays_frozen_after_manual_resize() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
        ..Default::default()
    };
    app.apply_ocr(&OcrText {
        text: "抓鬼 长寿村 12,34".into(),
        elapsed_secs: 0.1,
        regions: Vec::new(),
        notes: Vec::new(),
        ..Default::default()
    });
    app.set_map_window(true);
    let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    let spawn = app
        .map_window_spawn_layout("抓鬼")
        .expect("应建立抓鬼浮窗运行时状态");

    // 模拟用户拖动缩放后把新尺寸写回配置。builder 的几何值必须保持不动，
    // 否则下一帧会重新下发 InnerSize，和系统缩放互相拉扯，表现为窗口抖动。
    let layout = app.config.map_window_layouts.get_mut("抓鬼").unwrap();
    layout.width = spawn.width + 120.0;
    layout.height = spawn.height + 80.0;
    let layout = *layout;
    let mut output2 = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
    assert_eq!(
        app.map_window_spawn_layout("抓鬼"),
        Some(spawn),
        "用户调整后的尺寸不应反馈给 ViewportBuilder"
    );
    assert_eq!(
        app.config.map_window_layouts.get("抓鬼"),
        Some(&layout),
        "配置里的用户尺寸应保留"
    );
    output.textures_delta.clear();
    output2.textures_delta.clear();
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
        "更多",
        "显示方式",
        "工具",
        "设置",
        "帮助",
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
    assert!(
        texts.iter().any(|text| text == "更多"),
        "退出精简模式后顶栏菜单应恢复：{texts:?}"
    );
    output.textures_delta.clear();
}

fn result(generation: u64) -> OcrEvent {
    OcrEvent::Remote {
        generation,
        result: Ok(OcrText {
            text: "抓鬼 长寿村 12,34 鬼王 天宫 56,78".into(),
            elapsed_secs: 0.25,
            regions: Vec::new(),
            notes: Vec::new(),
            ..Default::default()
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
        regions: Vec::new(),
        notes: Vec::new(),
        ..Default::default()
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
                regions: Vec::new(),
                notes: Vec::new(),
                ..Default::default()
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
        regions: Vec::new(),
        notes: Vec::new(),
        ..Default::default()
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
                    regions: Vec::new(),
                    notes: Vec::new(),
                    ..Default::default()
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
    app.config.color_gate_enabled = true;
    app.save_active_profile();

    app.config.color_gate_enabled = false;
    let name = app.config.active_profile.clone();
    app.apply_profile(&name);
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
                regions: Vec::new(),
                notes: Vec::new(),
                ..Default::default()
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

/// 行分割调试窗口：拿到合成面板后应算出两行，并把行框 / 投影画出来。
#[test]
fn rows_debug_preview_shows_projection_rows() {
    let mut app = app();
    // 两行「方块字」，行间留足空隙，确保投影能切开。
    let panel = two_line_panel();
    app.show_rows_debug = true;
    app.set_rows_debug_panel(panel);
    let layout = app.rows_debug_layout.clone().expect("应算出分行结果");
    assert_eq!(layout.rows.len(), 2, "两行应切成两条带：{layout:?}");
    assert!(layout.reliable, "规整的两行应判可信");

    let ctx = app.ctx.clone();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            ..Default::default()
        },
        |ui| app.rows_debug_window_ui(ui),
    );
    let texts = collect_texts(&output.shapes);
    assert!(
        texts.iter().any(|text| text.contains("行数 2")),
        "应显示行数统计：{texts:?}"
    );
    assert!(app.rows_debug_texture.is_some(), "预览纹理应在渲染后建立");
    output.textures_delta.clear();
}

/// 归一化 + 混淆字符还原之后，解析器要能认出坐标和地图名。
#[test]
fn normalized_row_text_still_parses_tasks() {
    let app = app();
    let raw = "抓鬼 长寿村 ７９，２Ｇ 附近"; // 全角数字 + 字母 G 混淆
    let normalized = crate::ocr_normalize::snap_confusables(&crate::ocr_normalize::normalize(raw));
    assert_eq!(normalized, "抓鬼 长寿村 79,26 附近");
    let tasks = crate::core::parse_tasks(
        &normalized,
        &app.maps,
        crate::core::TaskFilter::KEYWORD_TASKS,
    );
    let task = tasks.first().expect("应解析出抓鬼任务");
    assert_eq!(task.map_name, "长寿村");
    assert_eq!((task.x, task.y), (79, 26));
}

/// 最近一次识别的行布局要留给侧边栏展示。
#[test]
fn apply_ocr_remembers_row_layout() {
    let mut app = app();
    let layout = crate::text_layout::rows(&two_line_panel(), Default::default());
    app.apply_ocr(&OcrText {
        text: "抓鬼 长寿村 12,34".into(),
        elapsed_secs: 0.1,
        regions: Vec::new(),
        notes: Vec::new(),
        layout: Some(layout),
        row_texts: vec!["抓鬼 长寿村 12,34".into()],
    });
    let remembered = app.last_ocr_layout.as_ref().expect("应保留行布局");
    assert_eq!(remembered.rows.len(), 2);
}

/// 两行方块字的面板：每行 12 个 8×12 的实心块，行间留 16px 空隙。
fn two_line_panel() -> image::RgbImage {
    let mut image = image::RgbImage::from_pixel(160, 80, image::Rgb([20, 24, 30]));
    for row in 0..2 {
        let top = 12 + row * 40;
        for index in 0..12u32 {
            let left = 8 + index * 12;
            for y in top..top + 14 {
                for x in left..left + 8 {
                    image.put_pixel(x, y, image::Rgb([240, 240, 240]));
                }
            }
        }
    }
    image
}

/// 「更多 → 工具」里要有行分割调试入口，点击按钮能切换开关。
#[test]
fn tools_menu_exposes_rows_debug_entry() {
    let mut app = app();
    let ctx = app.ctx.clone();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 600.0))),
            ..Default::default()
        },
        |ui| app.tools_menu_ui(ui, &ctx.clone()),
    );
    let texts = collect_texts(&output.shapes);
    output.textures_delta.clear();
    assert!(
        texts.iter().any(|text| text == "行分割调试…"),
        "工具菜单应提供行分割调试入口：{texts:?}"
    );
}

/// 没有选窗口时打开调试窗口只给提示，不崩；重复调用就是开关。
#[test]
fn toggle_rows_debug_flags_and_explains_missing_window() {
    let mut app = app();
    assert!(!app.show_rows_debug);
    app.toggle_rows_debug();
    assert!(app.show_rows_debug);
    assert!(
        app.rows_debug_message
            .as_deref()
            .unwrap_or_default()
            .contains("游戏窗口"),
        "没有选窗口时应提示先选窗口：{:?}",
        app.rows_debug_message
    );
    app.toggle_rows_debug();
    assert!(!app.show_rows_debug, "再点一次应关闭");
}
