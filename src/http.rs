use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, State};
use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use base64::Engine as _;
use qrcode::{EcLevel, QrCode};
use serde_json::{json, Map, Value};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeFile;

use crate::ocr::imgproc::{encode_jpeg, RgbImage};
use crate::ocr::{get_options_json, resolve_options, Engine};

const MAX_BODY: usize = 128 * 1024 * 1024;

pub fn router(engine: Arc<Engine>, web_index: PathBuf) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::OPTIONS,
        ])
        .allow_headers(Any)
        .expose_headers([header::CONTENT_LENGTH]);

    Router::new()
        .route("/", get(about))
        .route("/umiocr", get(about))
        .route("/argv", post(argv))
        .route("/api/ocr/get_options", get(get_options))
        .route("/api/ocr", post(ocr))
        .route("/api/qrcode", post(qrcode))
        .route_service("/index.html", ServeFile::new(web_index))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(cors)
        .with_state(engine)
}

/// 对应 Umi-OCR 的 "/" 与 "/umiocr"，返回项目标识字符串。
async fn about() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "Umi-OCR (Rust HTTP API) v2.1.2",
    )
        .into_response()
}

/// /argv：仅允许本机访问的跨进程命令接口，返回服务状态。
async fn argv(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: Bytes,
) -> Response {
    let ip = addr.ip();
    let local = ip.is_loopback() || ip.is_unspecified();
    if !local {
        return (
            StatusCode::UNAUTHORIZED,
            "Unauthorized access. Only local requests are allowed.\n此接口只允许本机访问。",
        )
            .into_response();
    }
    let _ = serde_json::from_slice::<Value>(&body);
    Json(json!({"code": 100, "data": "ok"})).into_response()
}

async fn get_options(State(engine): State<Arc<Engine>>) -> Json<Value> {
    Json(get_options_json(
        &engine.languages(),
        &engine.default_language(),
    ))
}

fn err_result(code: i32, message: impl Into<String>) -> Value {
    json!({"code": code, "data": message.into()})
}

/// POST /api/ocr
/// 请求：{"base64": "...", "options": {...}}
async fn ocr(State(engine): State<Arc<Engine>>, body: Bytes) -> Json<Value> {
    let value: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return Json(err_result(800, "请求无法解析为json。")),
    };
    let Some(obj) = value.as_object() else {
        return Json(err_result(800, "请求无法解析为json。"));
    };
    if obj.is_empty() {
        return Json(err_result(801, "请求为空。"));
    }

    let base64 = match obj.get("base64") {
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Json(err_result(802, "请求中 base64 字段必须为字符串。")),
        None => return Json(err_result(802, "请求中缺少 base64 字段。")),
    };

    let options_map: Map<String, Value> = match obj.get("options") {
        None => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Json(err_result(803, "请求中 options 字段必须为字典。")),
    };

    let opts = match resolve_options(&options_map, &engine.default_language()) {
        Ok(o) => o,
        Err(e) => {
            return Json(err_result(804, format!("options 解释失败。{e}")));
        }
    };

    let engine = engine.clone();
    let joined = tokio::task::spawn_blocking(move || engine.recognize(&base64, &opts)).await;
    match joined {
        Ok(result) => Json(result.to_json()),
        Err(e) => Json(err_result(300, format!("OCR 识别失败：{e}"))),
    }
}

/// POST /api/qrcode
/// 传入 base64 -> 识别二维码；传入 text -> 生成二维码 base64。
async fn qrcode(body: Bytes) -> Json<Value> {
    let value: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return Json(err_result(800, "请求无法解析为json。")),
    };
    let Some(obj) = value.as_object() else {
        return Json(err_result(800, "请求无法解析为json。"));
    };
    if obj.is_empty() {
        return Json(err_result(801, "请求为空。"));
    }

    if let Some(v) = obj.get("base64") {
        let text = v.as_str().unwrap_or("");
        return Json(qr_decode(text));
    }
    if let Some(v) = obj.get("text") {
        let text = v.as_str().unwrap_or("");
        let opts = obj
            .get("options")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        return Json(qr_encode(text, &opts));
    }
    Json(err_result(802, "指令中不存在 \"base64\" 或 \"text\""))
}

fn strip_data_uri(s: &str) -> &str {
    if let Some(i) = s.find(',') {
        if s.trim_start().to_ascii_lowercase().starts_with("data:") {
            return &s[i + 1..];
        }
    }
    s
}

/// 从 base64 图片中识别二维码。
fn qr_decode(b64: &str) -> Value {
    let raw = match base64::engine::general_purpose::STANDARD.decode(strip_data_uri(b64)) {
        Ok(raw) => raw,
        Err(e) => return err_result(200, format!("[Error] base64 解码失败：{e}")),
    };
    let img = match ::image::load_from_memory(&raw) {
        Ok(img) => img,
        Err(e) => return err_result(200, format!("[Error] 图片解码失败：{e}")),
    };

    let mut prepared = rqrr::PreparedImage::prepare(img.to_luma8());
    let grids = prepared.detect_grids();
    if grids.is_empty() {
        return err_result(200, "[Error] 未识别到二维码：no QR code found");
    }
    let mut last_err = String::new();
    for grid in &grids {
        match grid.decode() {
            Ok((_, text)) => {
                return json!({
                    "code": 100,
                    "data": [{"text": text, "format": "QRCode", "orientation": 0}],
                });
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    err_result(200, format!("[Error] 未识别到二维码：{last_err}"))
}

/// 将文本生成二维码，返回 JPEG 图片的 base64。
fn qr_encode(text: &str, opts: &Map<String, Value>) -> Value {
    let level = match opts
        .get("ec_level")
        .and_then(|v| v.as_f64())
        .filter(|n| (0.0..=3.0).contains(n))
        .map(|n| n as i32)
    {
        Some(0) => EcLevel::L,
        Some(1) => EcLevel::M,
        Some(2) => EcLevel::Q,
        Some(3) => EcLevel::H,
        _ => EcLevel::M,
    };
    let wanted = opts
        .get("w")
        .and_then(|v| v.as_f64())
        .map(|f| f as i64)
        .unwrap_or(0);

    let code = match QrCode::with_error_correction_level(text.as_bytes(), level) {
        Ok(code) => code,
        Err(e) => return err_result(200, format!("[Error] {e}")),
    };
    let modules = code.width();
    let colors = code.to_colors();
    let total_modules = modules + 8; // 四周边框各 4 个模块
    let scale = if wanted > 0 {
        ((wanted as usize) / total_modules).max(1)
    } else {
        4
    };
    let size = total_modules * scale;

    let mut img = RgbImage::new(size, size);
    for px in img.data.iter_mut() {
        *px = 255;
    }
    for y in 0..modules {
        for x in 0..modules {
            if colors[y * modules + x] == qrcode::Color::Dark {
                let ox = (x + 4) * scale;
                let oy = (y + 4) * scale;
                for dy in 0..scale {
                    for dx in 0..scale {
                        let i = ((oy + dy) * size + ox + dx) * 3;
                        img.data[i] = 0;
                        img.data[i + 1] = 0;
                        img.data[i + 2] = 0;
                    }
                }
            }
        }
    }

    match encode_jpeg(&img, 90) {
        Ok(bytes) => json!({
            "code": 100,
            "data": base64::engine::general_purpose::STANDARD.encode(bytes),
        }),
        Err(e) => err_result(200, format!("[Error] {e}")),
    }
}
