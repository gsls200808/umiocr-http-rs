use serde::Serialize;

/// 一个 OCR 文块。box 为四个顶点坐标（左上、右上、右下、左下）。
#[derive(Debug, Clone, Serialize)]
pub struct TextBlock {
    #[serde(rename = "box")]
    pub box_: Vec<[f64; 2]>,
    pub score: f64,
    pub text: String,
    pub end: String,
}

impl TextBlock {
    pub fn new(quad: [[f64; 2]; 4], score: f64, text: String) -> Self {
        Self {
            box_: quad.to_vec(),
            score,
            text,
            end: String::new(),
        }
    }
}

/// 忽略区域，由左上角 (x1,y1) 与右下角 (x2,y2) 确定。
#[derive(Debug, Clone, Copy)]
pub struct IgnoreRect {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

/// 一次 OCR 请求的完整参数。
#[derive(Debug, Clone)]
pub struct OcrOptions {
    pub language: String,
    pub angle: bool,
    pub max_side_len: i64,
    pub parser: String,
    pub data_format: String,
    pub ignore_area: Vec<IgnoreRect>,
}

/// 一种语言/模型库对应的四个模型文件。
#[derive(Debug, Clone)]
pub struct LangConfig {
    pub det: String,
    pub cls: String,
    pub rec: String,
    pub keys: String,
}

/// /api/ocr 的返回结构（与 Umi-OCR 一致，score 可选）。
#[derive(Debug)]
pub enum OcrData {
    Text(String),
    Blocks(Vec<TextBlock>),
}

#[derive(Debug)]
pub struct OcrResult {
    pub code: i32,
    pub data: OcrData,
    pub score: Option<f64>,
}

impl OcrResult {
    pub fn error(message: String) -> Self {
        Self {
            code: 300,
            data: OcrData::Text(message),
            score: None,
        }
    }

    pub fn empty() -> Self {
        Self {
            code: 101,
            data: OcrData::Text(String::new()),
            score: None,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        let data = match &self.data {
            OcrData::Text(t) => serde_json::Value::String(t.clone()),
            OcrData::Blocks(b) => serde_json::to_value(b).unwrap_or(serde_json::Value::Null),
        };
        let mut obj = serde_json::Map::new();
        obj.insert("code".into(), serde_json::json!(self.code));
        obj.insert("data".into(), data);
        if let Some(score) = self.score {
            obj.insert("score".into(), serde_json::json!(score));
        }
        serde_json::Value::Object(obj)
    }
}
