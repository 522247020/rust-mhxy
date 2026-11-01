//! 字库调试工具：给坐标数字建库、在截图上直接读字，不启动 UI。
//!
//! ```text
//! cargo run --bin font-tool -- build --samples samples --out fontlib.json
//! cargo run --bin font-tool -- read  --lib fontlib.json --image panel.png
//! cargo run --bin font-tool -- read  --lib fontlib.json --image line.png --line
//! ```
//!
//! 建库样本：`samples/` 下每个 `xxx.png` 是一行文字截图（黑字白底或原始截图都行），
//! 配同名 `xxx.txt` 写这行的准确文字，例如 `79,26`。

#[path = "../font_lib.rs"]
#[allow(dead_code)]
mod font_lib;

use font_lib::{GlyphLib, MatchConfig};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn usage() {
    eprintln!(
        "用法：
  font-tool build --samples <目录> --out <字库.json> [--cell 16x24]
  font-tool read  --lib <字库.json> --image <图片.png> [--line] [--crop x,y,w,h]
                  [--save-binary <二值化.png>]

build：目录下每个 xxx.png（一行文字的截图）+ 同名 xxx.txt（这行的准确文字）。
read ：默认按行切分整块面板；--line 表示整张图就是一行。"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("build") => match build(&args[1..]) {
            Ok(()) => {}
            Err(err) => exit_with(err),
        },
        Some("read") => match read(&args[1..]) {
            Ok(()) => {}
            Err(err) => exit_with(err),
        },
        Some("help") | Some("-h") | Some("--help") | None => usage(),
        Some(other) => {
            eprintln!("未知子命令：{other}");
            usage();
            std::process::exit(2);
        }
    }
}

fn exit_with(err: String) -> ! {
    eprintln!("错误：{err}");
    std::process::exit(1);
}

fn parse_flags(args: &[String]) -> HashMap<String, String> {
    let mut flags = HashMap::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let Some(name) = arg.strip_prefix("--") else {
            index += 1;
            continue;
        };
        let (name, value) = match name.split_once('=') {
            Some((name, value)) => (name.to_string(), Some(value.to_string())),
            None => (name.to_string(), None),
        };
        match value {
            Some(value) => {
                flags.insert(name, value);
                index += 1;
            }
            None => {
                let next = args.get(index + 1).filter(|next| !next.starts_with("--"));
                flags.insert(name, next.cloned().unwrap_or_else(|| "1".to_string()));
                index += if next.is_some() { 2 } else { 1 };
            }
        }
    }
    flags
}

fn required(flags: &HashMap<String, String>, name: &str) -> Result<String, String> {
    flags
        .get(name)
        .cloned()
        .ok_or_else(|| format!("缺少 --{name}"))
}

fn cell_size(flags: &HashMap<String, String>) -> Result<(u32, u32), String> {
    let Some(raw) = flags.get("cell") else {
        return Ok((font_lib::DEFAULT_CELL_W, font_lib::DEFAULT_CELL_H));
    };
    let (width, height) = raw
        .split_once(['x', 'X', '*'])
        .ok_or_else(|| format!("--cell 需要 宽x高，例如 16x24：{raw}"))?;
    let width = width.parse::<u32>().map_err(|err| format!("--cell 宽解析失败：{err}"))?;
    let height = height
        .parse::<u32>()
        .map_err(|err| format!("--cell 高解析失败：{err}"))?;
    if width == 0 || height == 0 {
        return Err("--cell 不能为 0".into());
    }
    Ok((width, height))
}

fn build(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let samples: PathBuf = required(&flags, "samples")?.into();
    let out: PathBuf = required(&flags, "out")?.into();
    let (cell_w, cell_h) = cell_size(&flags)?;
    let (lib, warnings) = font_lib::build_from_dir(&samples, cell_w, cell_h, MatchConfig::default())?;
    for warning in &warnings {
        eprintln!("警告：{warning}");
    }
    if lib.is_empty() {
        return Err("没有产出任何字模".into());
    }
    lib.save(&out)?;
    println!(
        "已写入 {}（{}x{}，{} 个字模）",
        out.display(),
        cell_w,
        cell_h,
        lib.len()
    );
    let mut counts = lib.label_counts();
    counts.sort_by(|a, b| a.0.cmp(&b.0));
    for (label, count) in counts {
        print!("{label}×{count}  ");
    }
    println!();
    Ok(())
}

fn read(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let lib_path: PathBuf = required(&flags, "lib")?.into();
    let image_path: PathBuf = required(&flags, "image")?.into();
    let lib = GlyphLib::load(&lib_path)?;
    if lib.is_empty() {
        return Err(format!("字库 {} 是空的", lib_path.display()));
    }
    let mut image = load_rgb(&image_path)?;
    if let Some(spec) = flags.get("crop") {
        let parts: Vec<u32> = spec
            .split(',')
            .filter_map(|part| part.trim().parse().ok())
            .collect();
        if parts.len() != 4 {
            return Err(format!("--crop 需要 x,y,w,h：{spec}"));
        }
        image = font_lib::crop(&image, parts[0], parts[1], parts[2], parts[3])?;
    }
    if let Some(path) = flags.get("save-binary") {
        let gray = font_lib::binarize(&image);
        gray.save(path)
            .map_err(|err| format!("写入二值化图 {path} 失败：{err}"))?;
        eprintln!("二值化结果已写到 {path}");
    }
    let single_line = flags.contains_key("line");
    let started = std::time::Instant::now();
    if single_line {
        let gray = font_lib::binarize(&image);
        let line = font_lib::read_line_gray(&gray, &lib, MatchConfig::default());
        print_line(&line);
    } else {
        for line in font_lib::read_lines(&image, &lib, MatchConfig::default()) {
            print_line(&line);
        }
    }
    eprintln!(
        "耗时 {:.2} ms，字库 {} 个字模",
        started.elapsed().as_secs_f64() * 1000.0,
        lib.len()
    );
    Ok(())
}

fn print_line(line: &font_lib::LineRead) {
    let mut detail = String::new();
    for glyph in &line.glyphs {
        detail.push_str(&format!(
            "{}({:.2}/{:.2}) ",
            glyph.label, glyph.score, glyph.margin
        ));
    }
    println!(
        "y={:<4} {:<12} {}{}",
        line.top,
        line.text,
        if line.confident { "" } else { "不可信 " },
        detail
    );
}

fn load_rgb(path: &Path) -> Result<image::RgbImage, String> {
    let image = image::open(path).map_err(|err| format!("读取图片 {} 失败：{err}", path.display()))?;
    Ok(image.to_rgb8())
}
