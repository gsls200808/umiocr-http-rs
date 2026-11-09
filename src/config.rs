use std::path::Path;

use serde::Deserialize;

fn default_port() -> u16 {
    1224
}
fn default_web_dir() -> String {
    "./web".to_string()
}
fn default_models() -> String {
    "models".to_string()
}
fn default_num_thread() -> usize {
    4
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerConfig {
    pub port: u16,
    pub web_dir: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            web_dir: default_web_dir(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OcrConfig {
    /// Umi-OCR 的 RapidOCR-json 插件目录（或任意包含 models 的目录）
    pub engine_dir: String,
    /// 模型子目录名，默认 models
    pub models: String,
    /// ONNX Runtime 每个会话的线程数
    pub num_thread: usize,
    /// rec 模型并发实例数，0 表示按 CPU 核心数自动取（最多 4）
    pub rec_pool: usize,
    /// 显式指定 onnxruntime.dll / libonnxruntime.so / libonnxruntime.dylib
    pub ort_lib: Option<String>,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            engine_dir: ".".to_string(),
            models: default_models(),
            num_thread: default_num_thread(),
            rec_pool: 0,
            ort_lib: None,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub ocr: OcrConfig,
}

impl AppConfig {
    /// 读取 YAML 配置；文件缺失或解析失败时回退默认值并打印警告。
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_yaml::from_str::<AppConfig>(&text) {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!(
                        "warning: 解析配置文件 {} 失败，将使用默认配置: {e}",
                        path.display()
                    );
                    AppConfig::default()
                }
            },
            Err(e) => {
                eprintln!(
                    "warning: 加载配置文件 {} 失败，将使用默认配置: {e}",
                    path.display()
                );
                AppConfig::default()
            }
        }
    }
}
