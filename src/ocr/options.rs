use serde_json::{json, Map, Value};

use super::types::{IgnoreRect, OcrOptions};

/// 解析并校验忽略区域，格式必须为 [[x1,y1],[x2,y2]]。
fn parse_ignore_area(v: &Value) -> Result<Vec<IgnoreRect>, String> {
    let arr = v
        .as_array()
        .ok_or_else(|| "tbpu.ignoreArea 必须为数组".to_string())?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let pair = item.as_array().ok_or_else(|| {
            "tbpu.ignoreArea 中，每一项的格式必须是 [[x1,y1],[x2,y2]]".to_string()
        })?;
        if pair.len() != 2 {
            return Err("tbpu.ignoreArea 中，每一项的格式必须是 [[x1,y1],[x2,y2]]".to_string());
        }
        let p1 = pair[0].as_array();
        let p2 = pair[1].as_array();
        let (p1, p2) = match (p1, p2) {
            (Some(a), Some(b)) if a.len() == 2 && b.len() == 2 => (a, b),
            _ => {
                return Err(
                    "tbpu.ignoreArea 中，每一项的格式必须是 [[x1,y1],[x2,y2]]".to_string(),
                )
            }
        };
        let nums: Option<Vec<f64>> = p1.iter().chain(p2.iter()).map(|v| v.as_f64()).collect();
        let nums = nums.ok_or_else(|| "tbpu.ignoreArea 中，坐标必须为数字".to_string())?;
        out.push(IgnoreRect {
            x1: nums[0],
            y1: nums[1],
            x2: nums[2],
            y2: nums[3],
        });
    }
    Ok(out)
}

/// 将请求中的 options 补全默认值，解析为强类型参数（与 Umi-OCR 的补充默认参数逻辑一致）。
pub fn resolve_options(
    raw: &Map<String, Value>,
    default_language: &str,
) -> Result<OcrOptions, String> {
    let mut opts = OcrOptions {
        language: default_language.to_string(),
        angle: false,
        max_side_len: 1024,
        parser: "multi_para".to_string(),
        data_format: "dict".to_string(),
        ignore_area: Vec::new(),
    };

    if let Some(Value::String(s)) = raw.get("ocr.language") {
        opts.language = s.clone();
    }
    if let Some(Value::Bool(b)) = raw.get("ocr.angle") {
        opts.angle = *b;
    }
    if let Some(v) = raw.get("ocr.maxSideLen") {
        if let Some(f) = v.as_f64() {
            opts.max_side_len = f as i64;
        }
    }
    if let Some(Value::String(s)) = raw.get("tbpu.parser") {
        opts.parser = s.clone();
    }
    if let Some(Value::String(s)) = raw.get("data.format") {
        opts.data_format = s.clone();
    }
    if let Some(v) = raw.get("tbpu.ignoreArea") {
        opts.ignore_area = parse_ignore_area(v)?;
    }
    Ok(opts)
}

/// 生成 /api/ocr/get_options 的返回内容，结构与 Umi-OCR 一致。
pub fn get_options_json(langs: &[String], default_language: &str) -> Value {
    let lang_opts: Vec<Value> = langs.iter().map(|l| json!([l, l])).collect();

    json!({
        "ocr.language": {
            "title": "语言/模型库",
            "default": default_language,
            "type": "enum",
            "optionsList": lang_opts,
        },
        "ocr.angle": {
            "title": "纠正文本方向",
            "default": false,
            "type": "boolean",
            "toolTip": "启用方向分类，识别倾斜或倒置的文本。可能降低识别速度。",
        },
        "ocr.maxSideLen": {
            "title": "限制图像边长",
            "default": 1024,
            "type": "enum",
            "optionsList": [[1024, "1024 （默认）"], [2048, "2048"], [4096, "4096"], [999999, "无限制"]],
            "toolTip": "将边长大于该值的图片进行压缩，可以提高识别速度。可能降低识别精度。",
        },
        "tbpu.parser": {
            "title": "排版解析方案",
            "toolTip": "按什么方式，解析和排序图片中的文字块",
            "default": "multi_para",
            "optionsList": [
                ["multi_para", "多栏-按自然段换行"],
                ["multi_line", "多栏-总是换行"],
                ["multi_none", "多栏-无换行"],
                ["single_para", "单栏-按自然段换行"],
                ["single_line", "单栏-总是换行"],
                ["single_none", "单栏-无换行"],
                ["single_code", "单栏-保留缩进"],
                ["none", "不做处理"],
            ],
        },
        "data.format": {
            "title": "数据返回格式",
            "toolTip": "返回值字典中，[\"data\"] 按什么格式表示 OCR 结果数据",
            "default": "dict",
            "optionsList": [
                ["dict", "含有位置等信息的原始字典"],
                ["text", "纯文本"],
            ],
        },
        "tbpu.ignoreArea": {
            "title": "忽略区域",
            "toolTip": "数组，每一项为[[左上角x,y],[右下角x,y]]。",
            "default": [],
        },
    })
}
