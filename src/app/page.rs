//! 侧栏页签。
//!
//! 以前页签是 `&'static str`，同一份「页面清单」散在菜单数组、主区分派、图标表、
//! 设置页分派和几处 `if tab == "..."` 里，加页 / 改名漏一处就是一处静默空白页。
//! 改成枚举后，这些地方都遍历 [`Page::PAGES`] / [`Page::ADVANCED`] 或走穷尽的
//! `match`，漏掉的分支编译器会直接报错。

/// 侧栏「页面」组里的页（按显示顺序）。
pub(super) const PAGES: &[Page] = &[
    Page::TaskMap,
    Page::GameWindow,
    Page::Region,
    Page::Mode,
    Page::TaskType,
    Page::Display,
    Page::Keymap,
    Page::Meditation,
    Page::Help,
];

/// 侧栏「高级设置」组里的页（按显示顺序）。
pub(super) const ADVANCED: &[Page] = &[Page::FlightFlags, Page::OcrMode, Page::Profiles, Page::Log];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) enum Page {
    /// 任务地图：任务面板 + 地图浮窗设置卡。
    #[default]
    TaskMap,
    GameWindow,
    Region,
    /// 识别模式：触发方式 / 间隔 / 实时。
    Mode,
    TaskType,
    /// 显示与布局：置顶、浮窗外观、高概率区域等。
    Display,
    Keymap,
    Meditation,
    Help,
    /// 合成旗编辑器（高级设置）。
    FlightFlags,
    /// 识别方式（高级设置）：本地 / 远端 PP-OCR。
    OcrMode,
    /// 方案管理（高级设置）。
    Profiles,
    Log,
}

impl Page {
    /// 页签文字：侧栏菜单和页标题都用它。
    pub(super) fn label(self) -> &'static str {
        match self {
            Page::TaskMap => "任务地图",
            Page::GameWindow => "游戏窗口",
            Page::Region => "识别区域",
            Page::Mode => "识别模式",
            Page::TaskType => "任务类型",
            Page::Display => "显示与布局",
            Page::Keymap => "按键映射",
            Page::Meditation => "打坐计时",
            Page::Help => "说明",
            Page::FlightFlags => "合成旗编辑器",
            Page::OcrMode => "识别方式",
            Page::Profiles => "方案管理",
            Page::Log => "日志",
        }
    }

    /// 页标题左侧的 Fluent 图标字形；没有图标字体时不会画出来。
    pub(super) fn glyph(self) -> &'static str {
        match self {
            Page::TaskMap => "\u{E816}",
            Page::GameWindow => "\u{E7B8}",
            Page::Mode => "\u{E721}",
            Page::TaskType => "\u{E716}",
            Page::Display => "\u{E740}",
            Page::Keymap => "\u{E765}",
            Page::Meditation => "\u{E823}",
            Page::OcrMode => "\u{E174}",
            Page::Profiles => "\u{E8F1}",
            Page::Log => "\u{E8B4}",
            Page::Region | Page::Help | Page::FlightFlags => "\u{E713}",
        }
    }
}
