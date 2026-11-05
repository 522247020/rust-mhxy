//! OCR：本地 PP-OCR（oar-ocr + ONNX Runtime）与远端 PP-OCR 服务。
//!
//! 本地识别有两条路：
//! 1. **分行路径（主）**：先用 `text_layout` 做投影分行，每行裁图后只跑识别模型
//!    （rec-only，跳过检测）。任务栏行数少、行距小，检测模型最容易在这里把相邻两行
//!    并成一个框，分行路径从根上避开这个问题。
//! 2. **检测路径（兜底）**：分行不可信（行数异常、行高差异过大）时回退到 PP-OCR 的
//!    「检测 + 识别」完整流程。
use image::{ImageEncoder, RgbImage};
use oar_ocr::core::config::OrtSessionConfig;
use oar_ocr::oarocr::OAROCRBuilder;
use oar_ocr::predictors::text_recognition::TextRecognitionPredictor;
use serde_json::Value;
use std::io::Cursor;
use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::{Duration, Instant};

pub const LOCAL_DET_MODEL: &str = "pp-ocrv5_mobile_det.onnx";
pub const LOCAL_REC_MODEL: &str = "pp-ocrv5_mobile_rec.onnx";
pub const LOCAL_DICT: &str = "ppocrv5_dict.txt";

/// RapidOCR 默认会占满所有 CPU 核心；限制线程数后识别更快、CPU 峰值也更低。
const LOCAL_INTRA_THREADS: usize = 4;
const REMOTE_TIMEOUT: Duration = Duration::from_secs(10);

/// 识别模型输入是 3x48x320，宽度超出会让整行被压扁；长行按这个宽高比切块。
const MAX_LINE_RATIO: f32 = 6.0;

#[derive(Clone, Debug, Default)]
pub struct OcrText {
    pub text: String,
    pub elapsed_secs: f64,
    /// 每个文本块的文字和位置。远端接口只回文本，所以可能是空的。
    pub regions: Vec<OcrRegion>,
    /// 归一化、分行、回退等处理产生的说明，由 UI 写详细日志。
    pub notes: Vec<String>,
    /// 本次用的行布局（分行路径算过时才有值），供调试预览使用。
    pub layout: Option<crate::text_layout::RowLayout>,
    /// 每一行归一化后的文本，与 `layout.rows` 下标一一对应。
    pub row_texts: Vec<String>,
}

/// 一个 OCR 检测框：内容、置信度、在原图（送 OCR 的那张图）里的矩形，
/// 以及它在 `OcrText::text` 里对应的字符区间（用于就地替换）。
#[derive(Clone, Debug, PartialEq)]
pub struct OcrRegion {
    pub text: String,
    pub score: f32,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    /// 在 `OcrText::text` 中的字符下标区间（左闭右开）。
    pub range: std::ops::Range<usize>,
}

/// 后台线程完成「截图 → 裁剪 → 预处理 → 放大」后的结果。
/// 把这些像素操作从 UI 线程拿走，避免每次识别时窗口拖动卡顿。
#[derive(Debug)]
pub struct Captured {
    pub full: RgbImage,
    pub panel: RgbImage,
    pub signature: u64,
}

#[derive(Debug)]
pub enum OcrEvent {
    /// 截图 / 预处理完成，UI 再决定是否真的送去 OCR。
    Captured {
        generation: u64,
        window_title: String,
        force: bool,
        result: Result<Captured, String>,
    },
    Local {
        generation: u64,
        result: Result<OcrText, String>,
    },
    Remote {
        generation: u64,
        result: Result<OcrText, String>,
    },
    /// 远端连接测试的结果（只用于日志提示）。
    Test { result: Result<OcrText, String> },
    /// 取点窗口里「测试 OCR」的结果，不参与正式识别流程。
    PanelTest { result: Result<OcrText, String> },
}

struct LocalJob {
    generation: u64,
    image: RgbImage,
    /// 测试任务不参与正式识别的合并与结果回写。
    test: bool,
    /// 高召回检测：降低检测阈值，宁可多出噪声框也别漏字（只影响兜底的检测路径）。
    high_recall: bool,
    /// 检测路径送模型前的放大倍数（分行路径不需要，识别模型自己会归一化行高）。
    upscale: u32,
}

/// 常驻线程：收到第一张截图时才加载模型，远端模式不占用模型内存。
pub struct LocalWorker {
    sender: Sender<LocalJob>,
}

impl LocalWorker {
    pub fn spawn(reply: Sender<OcrEvent>, ctx: egui::Context) -> LocalWorker {
        let (sender, receiver) = channel::<LocalJob>();
        thread::spawn(move || {
            let mut engine_normal = None;
            let mut engine_recall = None;
            let mut rows_engine: Option<RowsEngine> = None;
            let mut pending: std::collections::VecDeque<LocalJob> =
                std::collections::VecDeque::new();
            loop {
                let mut job = match pending.pop_front() {
                    Some(job) => job,
                    None => match receiver.recv() {
                        Ok(job) => job,
                        Err(_) => break,
                    },
                };
                if !job.test {
                    // 只合并连续的普通识别任务；测试任务保留到下一轮，避免界面卡在「识别中」。
                    while let Ok(newer) = receiver.try_recv() {
                        if newer.test {
                            pending.push_back(newer);
                        } else {
                            job = newer;
                        }
                    }
                }
                let result = (|| {
                    // 1) 分行路径：投影分行 → 每行 rec-only 识别。
                    let layout = crate::text_layout::rows(
                        &job.image,
                        crate::text_layout::RowConfig::default(),
                    );
                    if layout.reliable {
                        if rows_engine.is_none() {
                            rows_engine = Some(RowsEngine::new()?);
                        }
                        let mut ocr = rows_engine
                            .as_ref()
                            .expect("initialized rows engine")
                            .recognize(&job.image, &layout)?;
                        for note in &layout.notes {
                            ocr.notes.push(format!("分行：{note}"));
                        }
                        ocr.layout = Some(layout);
                        return Ok(ocr);
                    }

                    // 2) 兜底：行分割不可信时回退到「检测 + 识别」。
                    let slot = if job.high_recall {
                        &mut engine_recall
                    } else {
                        &mut engine_normal
                    };
                    if slot.is_none() {
                        *slot = Some(LocalEngine::new(job.high_recall)?);
                    }
                    let image = upscale(&job.image, job.upscale);
                    let mut ocr = slot
                        .as_ref()
                        .expect("initialized engine")
                        .recognize(&image)?;
                    ocr.notes.push(format!(
                        "行分割不可信（{} 行），已回退检测模型",
                        layout.rows.len()
                    ));
                    for note in &layout.notes {
                        ocr.notes.push(format!("分行：{note}"));
                    }
                    ocr.layout = Some(layout);
                    Ok(ocr)
                })();
                let event = if job.test {
                    OcrEvent::PanelTest { result }
                } else {
                    OcrEvent::Local {
                        generation: job.generation,
                        result,
                    }
                };
                if reply.send(event).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        LocalWorker { sender }
    }

    pub fn submit(
        &self,
        generation: u64,
        image: RgbImage,
        high_recall: bool,
        upscale: u32,
    ) -> Result<(), String> {
        self.sender
            .send(LocalJob {
                generation,
                image,
                test: false,
                high_recall,
                upscale,
            })
            .map_err(|_| "本地识别线程已退出，请重启程序".to_string())
    }

    /// 取点窗口的「测试 OCR」：不占用正式识别的 generation。
    pub fn submit_test(&self, image: RgbImage, upscale: u32) -> Result<(), String> {
        self.sender
            .send(LocalJob {
                generation: 0,
                image,
                test: true,
                high_recall: false,
                upscale,
            })
            .map_err(|_| "本地识别线程已退出，请重启程序".to_string())
    }
}

struct LocalEngine {
    ocr: oar_ocr::oarocr::OAROCR,
}

impl LocalEngine {
    fn new(high_recall: bool) -> Result<Self, String> {
        let session = OrtSessionConfig::default().with_intra_threads(LOCAL_INTRA_THREADS);
        let mut builder = OAROCRBuilder::new(
            bundled_model(LOCAL_DET_MODEL),
            bundled_model(LOCAL_REC_MODEL),
            bundled_model(LOCAL_DICT),
        )
        .ort_session(session);
        if high_recall {
            // 默认 0.3 / 0.6 / 1.5 偏保守；任务栏文字小、对比度低时容易漏检。
            let detection = oar_ocr::domain::tasks::text_detection::TextDetectionConfig {
                score_threshold: 0.2,
                box_threshold: 0.4,
                unclip_ratio: 1.8,
                ..Default::default()
            };
            builder = builder.text_detection_config(detection);
        }
        let ocr = builder
            .build()
            .map_err(|err| format!("加载本地 PP-OCR 模型失败：{err}"))?;
        Ok(Self { ocr })
    }

    fn recognize(&self, image: &RgbImage) -> Result<OcrText, String> {
        let started = Instant::now();
        let results = self
            .ocr
            .predict(vec![image.clone()])
            .map_err(|err| format!("本地 PP-OCR 推理失败：{err}"))?;
        let mut text = String::new();
        let mut regions = Vec::new();
        for (index, result) in results.iter().enumerate() {
            let (line_text, mut line_regions) = regions_to_text(result);
            if index > 0 {
                text.push('\n');
            }
            // `range` 是相对整段文本的字符下标，拼接后要整体平移。
            let offset = text.chars().count();
            for region in &mut line_regions {
                region.range = (region.range.start + offset)..(region.range.end + offset);
            }
            text.push_str(&line_text);
            regions.append(&mut line_regions);
        }
        Ok(OcrText {
            text,
            elapsed_secs: started.elapsed().as_secs_f64(),
            regions,
            notes: Vec::new(),
            layout: None,
            row_texts: Vec::new(),
        })
    }
}

/// 分行路径的识别引擎：只用识别模型（rec-only），不跑检测。
///
/// 行位置由 `text_layout` 的投影给出，这里负责：行图 → 长行切块 → 批量识别
/// → 文本归一化 → 拼成 `OcrText`。
struct RowsEngine {
    rec: TextRecognitionPredictor,
}

impl RowsEngine {
    fn new() -> Result<Self, String> {
        let session = OrtSessionConfig::default().with_intra_threads(LOCAL_INTRA_THREADS);
        let rec = TextRecognitionPredictor::builder()
            .dict_path(bundled_model(LOCAL_DICT))
            .with_ort_config(session)
            .build(bundled_model(LOCAL_REC_MODEL))
            .map_err(|err| format!("加载本地识别模型失败：{err}"))?;
        Ok(Self { rec })
    }

    fn recognize(
        &self,
        image: &RgbImage,
        layout: &crate::text_layout::RowLayout,
    ) -> Result<OcrText, String> {
        let started = Instant::now();
        let row_images =
            crate::text_layout::row_images(image, layout, crate::text_layout::RowConfig::default());

        // 每行按宽高比切块，拼成一个批次送一次识别（比逐行调用快很多）。
        let mut batch: Vec<RgbImage> = Vec::new();
        let mut chunk_counts: Vec<usize> = Vec::with_capacity(row_images.len());
        for row in &row_images {
            let gray = image::imageops::grayscale(row);
            let spans = crate::text_layout::chunks(&gray, MAX_LINE_RATIO);
            chunk_counts.push(spans.len());
            for (x0, x1) in spans {
                let width = x1.saturating_sub(x0).max(1);
                batch.push(image::imageops::crop_imm(row, x0, 0, width, row.height()).to_image());
            }
        }

        if batch.is_empty() {
            return Ok(OcrText {
                elapsed_secs: started.elapsed().as_secs_f64(),
                ..Default::default()
            });
        }
        let chunk_total = batch.len();
        let result = self
            .rec
            .predict(batch)
            .map_err(|err| format!("本地识别推理失败：{err}"))?;

        // 按行拼回文本（识别结果与输入顺序一一对应；防御性处理长度不一致）。
        let mut raw_rows: Vec<String> = vec![String::new(); row_images.len()];
        let mut scores: Vec<Vec<f32>> = vec![Vec::new(); row_images.len()];
        let mut cursor = 0usize;
        for (row_index, count) in chunk_counts.iter().enumerate() {
            for _ in 0..*count {
                if let Some(text) = result.texts.get(cursor) {
                    raw_rows[row_index].push_str(text);
                }
                if let Some(score) = result.scores.get(cursor) {
                    scores[row_index].push(*score);
                }
                cursor += 1;
            }
        }

        let mut ocr = OcrText {
            elapsed_secs: 0.0,
            notes: Vec::new(),
            layout: Some(layout.clone()),
            row_texts: Vec::with_capacity(row_images.len()),
            ..Default::default()
        };
        if result.texts.len() != chunk_total {
            ocr.notes.push(format!(
                "识别结果数量异常：{} 块输入，{} 条输出",
                chunk_total,
                result.texts.len()
            ));
        }

        for (row_index, raw) in raw_rows.iter().enumerate() {
            let normalized =
                crate::ocr_normalize::snap_confusables(&crate::ocr_normalize::normalize(raw));
            let text = normalized.trim().to_string();
            ocr.row_texts.push(text.clone());
            if text.is_empty() {
                continue;
            }
            let Some(row) = layout.rows.get(row_index) else {
                continue;
            };
            if !ocr.text.is_empty() {
                ocr.text.push('\n');
            }
            let start = ocr.text.chars().count();
            ocr.text.push_str(&text);
            let end = start + text.chars().count();
            let score = if scores[row_index].is_empty() {
                0.0
            } else {
                scores[row_index].iter().sum::<f32>() / scores[row_index].len() as f32
            };
            ocr.regions.push(OcrRegion {
                text,
                score,
                left: row.rect.x0 as f32,
                top: row.rect.y0 as f32,
                right: row.rect.x1 as f32,
                bottom: row.rect.y1 as f32,
                range: start..end,
            });
        }
        ocr.elapsed_secs = started.elapsed().as_secs_f64();
        Ok(ocr)
    }
}

/// 检测路径送模型前的放大（分行路径不需要：识别模型自己会把行高归一化到 48）。
fn upscale(image: &RgbImage, factor: u32) -> RgbImage {
    let factor = factor.clamp(1, 3);
    if factor <= 1 {
        return image.clone();
    }
    image::imageops::resize(
        image,
        image.width() * factor,
        image.height() * factor,
        image::imageops::FilterType::Lanczos3,
    )
}

/// 把检测框按位置整理成文本行：同一行的文字按 x 排序，行与行按 y 排序。
///
/// `all_text()` 按检测顺序返回，遇到两个任务条目、或者倾斜/换行时顺序可能乱掉。
/// 用检测框重新排一次，后续解析拿到的就是「从左到右、从上到下」的文本。
/// 一个检测框的文本和位置（组装成文本行时的中间结构）。
struct OcrItem {
    text: String,
    score: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

fn regions_to_text(result: &oar_ocr::oarocr::OAROCRResult) -> (String, Vec<OcrRegion>) {
    let items: Vec<OcrItem> = result
        .text_regions
        .iter()
        .filter_map(|region| {
            let text = region.text.as_deref()?.trim().to_string();
            if text.is_empty() {
                return None;
            }
            let mut top = f32::INFINITY;
            let mut bottom = f32::NEG_INFINITY;
            let mut left = f32::INFINITY;
            let mut right = f32::NEG_INFINITY;
            for point in &region.bounding_box.points {
                top = top.min(point.y);
                bottom = bottom.max(point.y);
                left = left.min(point.x);
                right = right.max(point.x);
            }
            top.is_finite().then_some(OcrItem {
                text,
                score: region.confidence.unwrap_or(0.0),
                left,
                top,
                right,
                bottom,
            })
        })
        .collect();
    assemble(items)
}

/// 排序 + 分行 + 拼文本，同时记录每个检测框在文本里的字符区间。
///
/// 拆出来是为了能单独测：拼文本的规则变了会直接影响 `parse_tasks` 能不能认出任务。
fn assemble(mut items: Vec<OcrItem>) -> (String, Vec<OcrRegion>) {
    items.sort_by(|a, b| a.top.total_cmp(&b.top).then(a.left.total_cmp(&b.left)));

    let mut lines: Vec<Vec<OcrItem>> = Vec::new();
    for item in items {
        let join = lines.last().is_some_and(|line| {
            let first = &line[0];
            let overlap = (first.bottom.min(item.bottom) - first.top.max(item.top)).max(0.0);
            let height = (first.bottom - first.top)
                .min(item.bottom - item.top)
                .max(1.0);
            overlap / height > 0.5
        });
        if join {
            if let Some(line) = lines.last_mut() {
                line.push(item);
            }
        } else {
            lines.push(vec![item]);
        }
    }

    let mut text = String::new();
    let mut regions = Vec::new();
    let mut char_index = 0usize;
    for (line_index, line) in lines.iter_mut().enumerate() {
        line.sort_by(|a, b| a.left.total_cmp(&b.left));
        if line_index > 0 {
            text.push('\n');
            char_index += 1;
        }
        for (item_index, item) in line.iter().enumerate() {
            if item_index > 0 {
                text.push(' ');
                char_index += 1;
            }
            let start = char_index;
            text.push_str(&item.text);
            char_index += item.text.chars().count();
            regions.push(OcrRegion {
                text: item.text.clone(),
                score: item.score,
                left: item.left,
                top: item.top,
                right: item.right,
                bottom: item.bottom,
                range: start..char_index,
            });
        }
    }
    (text, regions)
}

/// 优先使用随包携带的模型（`assets/models` 或 exe 旁的 `models`）；
/// 找不到时回退到模型名，交给 oar-ocr 自动下载。
fn bundled_model(name: &str) -> std::path::PathBuf {
    let mut candidates = vec![crate::config::assets_dir().join("models").join(name)];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("models").join(name));
        }
    }
    candidates
        .into_iter()
        .find(|path| path.exists())
        .unwrap_or_else(|| std::path::PathBuf::from(name))
}

/// 直接把 PNG 字节发给远端 PP-OCR 服务，取回纯文本。
pub fn remote_recognize(
    base_url: &str,
    token: &str,
    format: &str,
    png: &[u8],
) -> Result<OcrText, String> {
    if base_url.trim().is_empty() {
        return Err("远程识图需要填写 API 地址".into());
    }
    let url = build_ppocr_url(base_url, format)?;
    let (content_type, body) = build_ppocr_body(png, format)?;
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(REMOTE_TIMEOUT))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let started = Instant::now();
    let mut request = agent.post(&url).header("Content-Type", &content_type);
    if !token.trim().is_empty() {
        request = request.header("Authorization", format!("Bearer {}", token.trim()));
    }
    let response = request
        .send(body)
        .map_err(|err| format!("远程识图失败：{err}"))?;
    let mut response_body = response.into_body();
    let data = response_body
        .read_to_vec()
        .map_err(|err| format!("读取远程响应失败：{err}"))?;
    // 远端接口只回文本，没有行位置，所以 `regions` 为空。
    Ok(OcrText {
        text: crate::ocr_normalize::normalize(&extract_ocr_text(&data)),
        elapsed_secs: started.elapsed().as_secs_f64(),
        regions: Vec::new(),
        ..Default::default()
    })
}

/// 根据请求格式拼出 PP-OCR 接口地址（已带路径时先去掉再拼）。
pub fn build_ppocr_url(base_url: &str, format: &str) -> Result<String, String> {
    if format != "multipart" && format != "raw" {
        return Err(format!("不支持的 OCR 请求格式：{format}"));
    }
    let mut base = base_url.trim().trim_end_matches('/').to_string();
    for suffix in ["/ocr/raw", "/ocr"] {
        if base.ends_with(suffix) {
            base.truncate(base.len() - suffix.len());
            break;
        }
    }
    Ok(base + if format == "raw" { "/ocr/raw" } else { "/ocr" })
}

/// 构建 PP-OCR 请求体，返回 (Content-Type, body)。
pub fn build_ppocr_body(png: &[u8], format: &str) -> Result<(String, Vec<u8>), String> {
    match format {
        "raw" => Ok(("image/png".to_string(), png.to_vec())),
        "multipart" => {
            let boundary = format!(
                "{:032x}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0)
            );
            let head = format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"crop.png\"\r\nContent-Type: image/png\r\n\r\n"
            );
            let mut body = Vec::with_capacity(head.len() + png.len() + boundary.len() + 8);
            body.extend_from_slice(head.as_bytes());
            body.extend_from_slice(png);
            body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
            Ok((format!("multipart/form-data; boundary={boundary}"), body))
        }
        other => Err(format!("不支持的 OCR 请求格式：{other}")),
    }
}

pub fn encode_png(image: &RgbImage) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::new();
    image::codecs::png::PngEncoder::new(Cursor::new(&mut buffer))
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|err| format!("PNG 编码失败：{err}"))?;
    Ok(buffer)
}

/// 解析 PP-OCR 接口响应（兼容多种响应结构），提取纯文本。
pub fn extract_ocr_text(data: &[u8]) -> String {
    match serde_json::from_slice::<Value>(data) {
        Ok(value) => extract_text(&value),
        Err(_) => String::from_utf8_lossy(data).trim().to_string(),
    }
}

fn extract_text(payload: &Value) -> String {
    match payload {
        Value::String(text) => text.trim().to_string(),
        Value::Object(map) => {
            for key in ["rec_texts", "texts", "rec_text", "text"] {
                match map.get(key) {
                    Some(Value::Array(items)) => {
                        let lines: Vec<String> = items
                            .iter()
                            .filter_map(|item| item.as_str())
                            .map(str::trim)
                            .filter(|item| !item.is_empty())
                            .map(str::to_string)
                            .collect();
                        if !lines.is_empty() {
                            return lines.join("\n");
                        }
                    }
                    Some(Value::String(text)) if !text.trim().is_empty() => {
                        return text.trim().to_string();
                    }
                    _ => {}
                }
            }
            for key in ["results", "result", "data", "output"] {
                if let Some(child) = map.get(key) {
                    let extracted = extract_text(child);
                    if !extracted.is_empty() {
                        return extracted;
                    }
                }
            }
            let mut lines = Vec::new();
            collect_lines(payload, &mut lines);
            lines.join("\n")
        }
        _ => {
            let mut lines = Vec::new();
            collect_lines(payload, &mut lines);
            lines.join("\n")
        }
    }
}

fn collect_lines(node: &Value, out: &mut Vec<String>) {
    match node {
        Value::Object(map) => {
            if let Some(Value::String(text)) = map.get("text") {
                if !text.trim().is_empty() {
                    out.push(text.trim().to_string());
                    return;
                }
            }
            for child in map.values() {
                collect_lines(child, out);
            }
        }
        Value::Array(items) => {
            // [text, score] 形式
            if items.len() == 2 {
                if let (Some(text), Some(score)) = (items[0].as_str(), items[1].as_f64()) {
                    let _ = score;
                    if !text.trim().is_empty() {
                        out.push(text.trim().to_string());
                        return;
                    }
                }
            }
            // [[box...], [text, score]] 形式
            if items.len() >= 2 {
                if let (Some(Value::Array(_)), Some(Value::Array(second))) =
                    (items.first(), items.get(1))
                {
                    if let Some(text) = second.first().and_then(|value| value.as_str()) {
                        if !text.trim().is_empty() {
                            out.push(text.trim().to_string());
                            return;
                        }
                    }
                }
            }
            for child in items {
                collect_lines(child, out);
            }
        }
        Value::String(text) if !text.trim().is_empty() => {
            out.push(text.trim().to_string());
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_ppocr_urls() {
        assert_eq!(
            build_ppocr_url("http://host:8008", "multipart").unwrap(),
            "http://host:8008/ocr"
        );
        assert_eq!(
            build_ppocr_url("http://host:8008/ocr", "raw").unwrap(),
            "http://host:8008/ocr/raw"
        );
    }

    #[test]
    fn extracts_text_from_common_shapes() {
        assert_eq!(extract_ocr_text(br#"{"rec_texts":["a","b"]}"#), "a\nb");
        assert_eq!(
            extract_ocr_text(br#"{"result":[[[0,0],[1,1]],["hello",0.9]]}"#),
            "hello"
        );
        assert_eq!(extract_ocr_text(b"plain text"), "plain text");
    }

    /// 组装规则：同一行按 x 排序用空格连，换行用 \n，并记录每个框的字符区间。
    /// 这条规则变了会直接影响 `parse_tasks` 的解析结果，所以单独测。
    #[test]
    fn assembles_regions_into_lines_with_ranges() {
        let item = |text: &str, left: f32, top: f32| OcrItem {
            text: text.to_string(),
            score: 0.9,
            left,
            top,
            right: left + 80.0,
            bottom: top + 24.0,
        };
        // 故意乱序传入：右边那块先给。
        let (text, regions) = assemble(vec![
            item("79,26", 300.0, 100.0),
            item("抓鬼", 10.0, 100.0),
            item("鬼王 天宫", 10.0, 200.0),
        ]);
        assert_eq!(text, "抓鬼 79,26\n鬼王 天宫");
        assert_eq!(regions.len(), 3);
        // 区间要能直接从文本里切回来。
        let slice = |range: std::ops::Range<usize>| -> String {
            text.chars().skip(range.start).take(range.len()).collect()
        };
        assert_eq!(slice(regions[0].range.clone()), "抓鬼");
        assert_eq!(slice(regions[1].range.clone()), "79,26");
        assert_eq!(slice(regions[2].range.clone()), "鬼王 天宫");
        assert!((regions[1].left - 300.0).abs() < 1e-6);
    }

    #[test]
    fn assemble_of_nothing_is_empty() {
        let (text, regions) = assemble(Vec::new());
        assert!(text.is_empty());
        assert!(regions.is_empty());
    }

    #[test]
    #[ignore = "首次运行会下载约 25MB 模型"]
    fn local_engine_smoke() {
        let engine = LocalEngine::new(false).expect("加载本地 PP-OCR 模型");
        let image = RgbImage::from_pixel(64, 64, image::Rgb([255, 255, 255]));
        engine.recognize(&image).expect("本地 PP-OCR 推理");
    }

    /// 端到端：本地 OCR 识别任务文字并解析出抓鬼 / 鬼王坐标。
    #[test]
    #[ignore = "需要 MHXY_OCR_TEST_IMAGE 指向一张任务文字截图"]
    fn local_engine_parses_task_text() {
        let path = std::env::var("MHXY_OCR_TEST_IMAGE")
            .unwrap_or_else(|_| "/tmp/mhxy-ocr/panel.png".to_string());
        let Ok(image) = image::open(&path) else {
            eprintln!("跳过：找不到测试图片 {path}");
            return;
        };
        let engine = LocalEngine::new(false).expect("加载本地 PP-OCR 模型");
        let result = engine
            .recognize(&image.to_rgb8())
            .expect("本地 PP-OCR 推理");
        println!("OCR 文本：\n{}", result.text);
        let maps = crate::core::load_maps(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/maps.json"
        )))
        .expect("加载 maps.json");
        let tasks =
            crate::core::parse_tasks(&result.text, &maps, crate::core::TaskFilter::KEYWORD_TASKS);
        println!("解析结果：{tasks:?}");
        assert!(tasks
            .iter()
            .any(|task| task.task_type == "抓鬼" && task.map_name == "长寿村"));
        assert!(tasks
            .iter()
            .any(|task| task.task_type == "鬼王" && task.map_name == "天宫"));
    }
}
