//! 梦幻西游抓鬼/鬼王任务坐标辅助（Rust + egui）。
mod app;
mod capture;
mod config;
mod core;
mod gate;
mod ocr;

use std::sync::Arc;

fn main() -> eframe::Result {
    let config = config::Config::load(&config::default_config_path());
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("梦幻西游任务坐标辅助（rust-mhxy）")
        .with_inner_size([1000.0, 740.0])
        .with_min_inner_size([760.0, 540.0]);
    if config.on_top {
        viewport = viewport.with_always_on_top();
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "rust-mhxy",
        options,
        Box::new(|cc| {
            install_fonts(&cc.egui_ctx);
            Ok(Box::new(app::MhxyApp::new(cc)))
        }),
    )
}

/// egui 自带的字体没有中文字形，这里从系统里找一个 CJK 字体装上。
fn install_fonts(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        // Windows
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/msyh.ttf",
        "C:/Windows/Fonts/simhei.ttf",
        "C:/Windows/Fonts/simsun.ttc",
        // Linux
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
        "/usr/share/fonts/wenquanyi/wqy-zenhei/wqy-zenhei.ttc",
        // macOS
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Medium.ttc",
    ];
    let Some(bytes) = CANDIDATES
        .iter()
        .find_map(|path| std::fs::read(path).ok())
    else {
        eprintln!("未找到可用的中文字体，界面中文可能显示为方块");
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("cjk".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "cjk".to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push("cjk".to_owned());
    ctx.set_fonts(fonts);
}
