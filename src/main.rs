//! 梦幻西游抓鬼/鬼王任务坐标辅助（Rust + egui）。
// Windows 使用 GUI 子系统，双击启动时不创建控制台窗口。
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod capture;
mod config;
mod core;
mod flight_flags;
mod gate;
mod hotkey;
mod meditation;
mod ocr;
mod preprocess;
mod theme;
mod update;

use std::sync::Arc;

pub const APP_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));
pub const APP_REPO_URL: &str = "https://github.com/522247020/rust-mhxy";

fn main() -> eframe::Result {
    if !acquire_single_instance() {
        return Ok(());
    }
    let config = config::Config::load(&config::default_config_path());
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(format!("梦幻西游任务坐标辅助（rust-mhxy {APP_VERSION}）"))
        .with_inner_size([1120.0, 800.0])
        .with_min_inner_size([760.0, 540.0])
        .with_icon(app_icon());
    if config.on_top {
        viewport = viewport.with_always_on_top();
    }
    let theme_mode = config.theme.clone();
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "rust-mhxy",
        options,
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx);
            let light = theme::resolve_light(&theme_mode, &cc.egui_ctx);
            theme::install(&cc.egui_ctx, light);
            Ok(Box::new(app::MhxyApp::new(cc)))
        }),
    )
}

/// 单实例：两个进程会抢同一组全局快捷键、重复跑 OCR、互相覆盖配置。
/// 已经有一个实例时提示并直接退出。
#[cfg(windows)]
fn acquire_single_instance() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    let name: Vec<u16> = "Local\\rust-mhxy-single-instance"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        match CreateMutexW(None, true, PCWSTR(name.as_ptr())) {
            Ok(handle) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    show_already_running();
                    false
                } else {
                    // HANDLE 没有 Drop，不调用 CloseHandle 就会一直持有到进程结束。
                    let _ = handle;
                    true
                }
            }
            // 创建失败就不拦着启动，避免因为权限问题完全打不开。
            Err(_) => true,
        }
    }
}

#[cfg(windows)]
fn show_already_running() {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
    let text: Vec<u16> = "rust-mhxy 已经在运行。\n请在任务栏或托盘切换到已打开的窗口。"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let title: Vec<u16> = "rust-mhxy"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

#[cfg(not(windows))]
fn acquire_single_instance() -> bool {
    true
}

fn app_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/app-icon.png"))
        .expect("内置应用图标必须是有效的 PNG")
}

/// 把字体文件只读映射到内存。字体页由系统按需调入，属于文件缓存而不是进程私有内存，
/// 比读入 Vec 再交给 epaint 占用低很多。映射失败时回退到读入内存。
fn load_font(path: &std::path::Path) -> Option<&'static [u8]> {
    if let Ok(file) = std::fs::File::open(path) {
        // SAFETY: 只读映射系统字体文件，仅用于字形渲染，映射保持到进程退出。
        if let Ok(map) = unsafe { memmap2::Mmap::map(&file) } {
            let leaked: &'static mut memmap2::Mmap = Box::leak(Box::new(map));
            return Some(&leaked[..]);
        }
    }
    let bytes = std::fs::read(path).ok()?;
    Some(Box::leak(bytes.into_boxed_slice()))
}

/// 系统里是否找到了可用的粗体中文字体。
static BOLD_FONT_AVAILABLE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 悬浮窗等地方用真实粗体；没有粗体字体时退回叠加绘制。
pub fn has_bold_font() -> bool {
    BOLD_FONT_AVAILABLE.load(std::sync::atomic::Ordering::Relaxed)
}

/// 粗体中文字体，找不到时退回普通字体。
const BOLD_CANDIDATES: &[&str] = &[
    "C:/Windows/Fonts/msyhbd.ttc",
    "C:/Windows/Fonts/msyhbd.ttf",
    "C:/Windows/Fonts/simhei.ttf",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Bold.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc",
    "/System/Library/Fonts/PingFang.ttc",
];

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
    let Some(font) = CANDIDATES
        .iter()
        .find_map(|path| load_font(std::path::Path::new(path)))
    else {
        eprintln!("未找到可用的中文字体，界面中文可能显示为方块");
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    // epaint 会为 FontData 的字体数据再准备一份 Blob；用 'static 切片可以避免复制。
    fonts.font_data.insert(
        "cjk".to_owned(),
        Arc::new(egui::FontData::from_static(font)),
    );
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

    // 粗体单独做成 "bold" 字体族：有系统粗体就用它，否则指向普通字体，
    // 调用方再决定要不要叠加绘制。
    let bold = BOLD_CANDIDATES
        .iter()
        .find_map(|path| load_font(std::path::Path::new(path)));
    let bold_name = match bold {
        Some(bytes) => {
            fonts.font_data.insert(
                "cjk_bold".to_owned(),
                Arc::new(egui::FontData::from_static(bytes)),
            );
            BOLD_FONT_AVAILABLE.store(true, std::sync::atomic::Ordering::Relaxed);
            "cjk_bold".to_owned()
        }
        None => "cjk".to_owned(),
    };
    let mut bold_family = vec![bold_name];
    bold_family.extend(
        fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    fonts
        .families
        .insert(egui::FontFamily::Name("bold".into()), bold_family);

    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 系统有粗体字体时，`bold` 字体族必须比常规字体更宽。
    #[test]
    fn bold_family_uses_real_bold_font_when_available() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mut output = ctx.run_ui(Default::default(), |_ui| {});
        output.textures_delta.clear();
        if !has_bold_font() {
            eprintln!("系统没有可用的粗体中文字体，跳过");
            return;
        }
        let regular = ctx.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(
                    "180".into(),
                    egui::FontId::proportional(60.0),
                    egui::Color32::WHITE,
                )
                .size()
        });
        let bold = ctx.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(
                    "180".into(),
                    egui::FontId::new(60.0, egui::FontFamily::Name("bold".into())),
                    egui::Color32::WHITE,
                )
                .size()
        });
        assert!(
            bold.x > regular.x,
            "粗体应比常规字体更宽：regular={regular:?} bold={bold:?}"
        );
    }

    /// 字体改成零拷贝加载后，中文字形必须仍然可用。
    #[test]
    fn bundled_font_covers_ui_and_map_characters() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let sample = "梦幻西游任务坐标辅助抓鬼鬼王长寿村天宫普陀山五庄观附近等待识别";
        let font_id = egui::FontId::proportional(14.0);
        // 字体在第一次渲染帧后可用。
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label(sample);
        });
        let covered = ctx.fonts_mut(|fonts| fonts.has_glyphs(&font_id, sample));
        output.textures_delta.clear();
        assert!(covered, "界面或地图字符缺字形，字体加载异常");
    }
}
