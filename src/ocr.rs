//! OCR：本地 PP-OCR（oar-ocr + ONNX Runtime）与远端 PP-OCR 服务。
use image::{ImageEncoder, RgbImage};
use oar_ocr::core::config::OrtSessionConfig;
use oar_ocr::oarocr::OAROCRBuilder;
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

#[derive(Clone, Debug)]
pub struct OcrText {
    pub text: String,
    pub elapsed_secs: f64,
}

#[derive(Debug)]
pub enum OcrEvent {
    Local {
        generation: u64,
        result: Result<OcrText, String>,
    },
    Remote {
        generation: u64,
        result: Result<OcrText, String>,
    },
    /// 远端连接测试的结果（只用于日志提示）。
    Test {
        result: Result<OcrText, String>,
    },
}

struct LocalJob {
    generation: u64,
    image: RgbImage,
}

/// 常驻线程：启动时加载一次本地 OCR 模型，之后循环处理截图。
pub struct LocalWorker {
    sender: Sender<LocalJob>,
}

impl LocalWorker {
    pub fn spawn(reply: Sender<OcrEvent>) -> LocalWorker {
        let (sender, receiver) = channel::<LocalJob>();
        thread::spawn(move || {
            let engine = match LocalEngine::new() {
                Ok(engine) => Ok(engine),
                Err(err) => Err(err),
            };
            while let Ok(job) = receiver.recv() {
                let result = match &engine {
                    Ok(engine) => engine.recognize(&job.image),
                    Err(err) => Err(err.clone()),
                };
                let _ = reply.send(OcrEvent::Local {
                    generation: job.generation,
                    result,
                });
            }
        });
        LocalWorker { sender }
    }

    pub fn submit(&self, generation: u64, image: RgbImage) {
        let _ = self.sender.send(LocalJob { generation, image });
    }
}

struct LocalEngine {
    ocr: oar_ocr::oarocr::OAROCR,
}

impl LocalEngine {
    fn new() -> Result<Self, String> {
        let session = OrtSessionConfig::default().with_intra_threads(LOCAL_INTRA_THREADS);
        let ocr = OAROCRBuilder::new(
            bundled_model(LOCAL_DET_MODEL),
            bundled_model(LOCAL_REC_MODEL),
            bundled_model(LOCAL_DICT),
        )
        .ort_session(session)
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
        let text = results
            .iter()
            .flat_map(|result| result.all_text())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(OcrText {
            text,
            elapsed_secs: started.elapsed().as_secs_f64(),
        })
    }
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
    Ok(OcrText {
        text: extract_ocr_text(&data),
        elapsed_secs: started.elapsed().as_secs_f64(),
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
            let boundary = format!("{:032x}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0));
            let head = format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"crop.png\"\r\nContent-Type: image/png\r\n\r\n"
            );
            let mut body = Vec::with_capacity(head.len() + png.len() + boundary.len() + 8);
            body.extend_from_slice(head.as_bytes());
            body.extend_from_slice(png);
            body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
            Ok((
                format!("multipart/form-data; boundary={boundary}"),
                body,
            ))
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
                if let (Some(Value::Array(_)), Some(Value::Array(second))) = (items.first(), items.get(1)) {
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
        Value::String(text) => {
            if !text.trim().is_empty() {
                out.push(text.trim().to_string());
            }
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

    #[test]
    #[ignore = "首次运行会下载约 25MB 模型"]
    fn local_engine_smoke() {
        let engine = LocalEngine::new().expect("加载本地 PP-OCR 模型");
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
        let engine = LocalEngine::new().expect("加载本地 PP-OCR 模型");
        let result = engine.recognize(&image.to_rgb8()).expect("本地 PP-OCR 推理");
        println!("OCR 文本：\n{}", result.text);
        let maps = crate::core::load_maps(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/maps.json"
        )))
        .expect("加载 maps.json");
        let tasks = crate::core::parse_tasks(&result.text, &maps, true);
        println!("解析结果：{tasks:?}");
        assert!(tasks
            .iter()
            .any(|task| task.task_type == "抓鬼" && task.map_name == "长寿村"));
        assert!(tasks
            .iter()
            .any(|task| task.task_type == "鬼王" && task.map_name == "天宫"));
    }
}
