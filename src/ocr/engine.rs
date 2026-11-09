use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{anyhow, bail, Context, Result};
use ort::environment::Environment;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;

use super::det::{db_post_process, filter_boxes};
use super::imgproc::{
    decode_base64_image, resize, resize_norm, rotate180, to_chw, warp_crop, RgbImage,
};
use super::tbpu::{apply_ignore_area, apply_parser, get_data_text};
use super::types::{LangConfig, OcrData, OcrOptions, OcrResult, TextBlock};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 某一语言/模型库对应的 ONNX 会话与识别字典。
/// recs 是 rec 模型池：并发识别时每个任务占用一份独立实例，避免任何共享状态。
struct ModelSet {
    det: Mutex<Session>,
    cls: Mutex<Session>,
    recs: Mutex<Vec<Session>>,
    chars: Arc<Vec<String>>,
}

struct Job {
    quad: [[f64; 2]; 4],
    crop: RgbImage,
}

pub struct Engine {
    dir: PathBuf,
    models: String,
    num_thread: usize,
    rec_pool: usize,
    env: Environment,
    langs: HashMap<String, LangConfig>,
    lang_order: Vec<String>,
    models_cache: Mutex<HashMap<String, Arc<ModelSet>>>,
    ocr_lock: Mutex<()>,
}

impl Engine {
    /// 创建引擎管理器，并读取 Umi-OCR 的 models/configs.txt 构建语言映射。
    pub fn new(
        dir: PathBuf,
        models: String,
        num_thread: usize,
        rec_pool: usize,
        env: Environment,
    ) -> Result<Self> {
        let mut engine = Self {
            dir,
            models: if models.is_empty() {
                "models".to_string()
            } else {
                models
            },
            num_thread: if num_thread == 0 { 4 } else { num_thread },
            rec_pool,
            env,
            langs: HashMap::new(),
            lang_order: Vec::new(),
            models_cache: Mutex::new(HashMap::new()),
            ocr_lock: Mutex::new(()),
        };
        engine.load_configs()?;
        Ok(engine)
    }

    /// 解析 configs.txt：每 5 行为一组（标题、det、cls、rec、keys），组间空行分隔。
    fn load_configs(&mut self) -> Result<()> {
        let path = self.dir.join(&self.models).join("configs.txt");
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("读取模型配置失败 {}", path.display()))?;
        let content = data.replace("\r\n", "\n");
        for part in content.split("\n\n") {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let lines: Vec<&str> = part.lines().collect();
            if lines.len() < 5 {
                continue;
            }
            let title = lines[0].trim().to_string();
            if title.is_empty() {
                continue;
            }
            let lc = LangConfig {
                det: lines[1].trim().to_string(),
                cls: lines[2].trim().to_string(),
                rec: lines[3].trim().to_string(),
                keys: lines[4].trim().to_string(),
            };
            if !self.langs.contains_key(&title) {
                self.lang_order.push(title.clone());
            }
            self.langs.insert(title, lc);
        }
        if self.lang_order.is_empty() {
            bail!("模型配置为空，未找到任何语言/模型库");
        }
        Ok(())
    }

    pub fn default_language(&self) -> String {
        self.lang_order
            .first()
            .cloned()
            .unwrap_or_else(|| "简体中文".to_string())
    }

    pub fn languages(&self) -> Vec<String> {
        self.lang_order.clone()
    }

    fn load_session(&self, path: &Path) -> Result<Session> {
        let builder = Session::builder(&self.env).map_err(|e| anyhow!("{e}"))?;
        let mut builder = builder
            .with_optimization_level(GraphOptimizationLevel::All)
            .map_err(|e| anyhow!("{e}"))?;
        builder = builder
            .with_intra_threads(self.num_thread)
            .map_err(|e| anyhow!("{e}"))?;
        builder
            .commit_from_file(path)
            .map_err(|e| anyhow!("{e}"))
            .with_context(|| format!("加载模型 {}", path.display()))
    }

    fn load_models(&self, lc: &LangConfig) -> Result<ModelSet> {
        let dir = self.dir.join(&self.models);
        let det = self
            .load_session(&dir.join(&lc.det))
            .context("加载 det 模型")?;
        let cls = self
            .load_session(&dir.join(&lc.cls))
            .context("加载 cls 模型")?;

        let mut n_rec = if self.rec_pool > 0 {
            self.rec_pool
        } else {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
                .min(4)
        };
        if n_rec < 1 {
            n_rec = 1;
        }
        let mut recs = Vec::with_capacity(n_rec);
        for _ in 0..n_rec {
            recs.push(
                self.load_session(&dir.join(&lc.rec))
                    .context("加载 rec 模型")?,
            );
        }

        let chars = load_dict(&dir.join(&lc.keys))?;
        Ok(ModelSet {
            det: Mutex::new(det),
            cls: Mutex::new(cls),
            recs: Mutex::new(recs),
            chars: Arc::new(chars),
        })
    }

    /// 按语言懒加载并缓存 ONNX 模型集。
    fn models_for(&self, lang: &str) -> Result<Arc<ModelSet>> {
        let lc = self
            .langs
            .get(lang)
            .cloned()
            .ok_or_else(|| anyhow!("未知的语言/模型库: {lang}"))?;

        let mut cache = lock(&self.models_cache);
        if let Some(m) = cache.get(lang) {
            return Ok(m.clone());
        }
        let m = Arc::new(self.load_models(&lc)?);
        cache.insert(lang.to_string(), m.clone());
        Ok(m)
    }

    /// 预热默认语言模型，降低首次请求延迟。
    pub fn warmup(&self) -> Result<()> {
        let lang = self.default_language();
        self.models_for(&lang).map(|_| ())
    }

    /// 识别一张 base64 图像，并完成排版解析与格式化输出。
    pub fn recognize(&self, image_base64: &str, opts: &OcrOptions) -> OcrResult {
        let img = match decode_base64_image(image_base64) {
            Ok(img) => img,
            Err(e) => return OcrResult::error(format!("OCR 识别失败：图像解码失败：{e:#}")),
        };
        let model = match self.models_for(&opts.language) {
            Ok(model) => model,
            Err(e) => return OcrResult::error(format!("OCR 识别失败：{e:#}")),
        };

        let _guard = lock(&self.ocr_lock);
        match recognize_with(&model, &img, opts) {
            Ok(blocks) => finish_ocr(blocks, opts),
            Err(e) => OcrResult::error(format!("OCR 识别失败：{e:#}")),
        }
    }
}

/// 读取字典，构造 CTC 字符表：["blank"] + 字典各行 + [" "]。
fn load_dict(path: &Path) -> Result<Vec<String>> {
    let data = std::fs::read_to_string(path).with_context(|| format!("读取字典 {}", path.display()))?;
    let data = data.replace("\r\n", "\n");
    let mut lines: Vec<&str> = data.split('\n').collect();
    while matches!(lines.last(), Some(&"")) {
        lines.pop();
    }
    let mut chars = Vec::with_capacity(lines.len() + 2);
    chars.push("blank".to_string());
    chars.extend(lines.iter().map(|s| (*s).to_string()));
    chars.push(" ".to_string());
    Ok(chars)
}

/// 长边超过 max_side_len 时按比例缩小，并把边长取整到 32 的倍数。
fn preprocess(img: &RgbImage, max_side_len: i64) -> (Cow<'_, RgbImage>, f64, f64) {
    let (h, w) = (img.h, img.w);
    if max_side_len > 0 && h.max(w) as i64 > max_side_len {
        let ratio = max_side_len as f64 / h.max(w) as f64;
        let rh = ((h as f64 * ratio / 32.0).round() * 32.0) as usize;
        let rw = ((w as f64 * ratio / 32.0).round() * 32.0) as usize;
        let rh = rh.max(32);
        let rw = rw.max(32);
        let out = resize(img, rw, rh);
        (
            Cow::Owned(out),
            h as f64 / rh as f64,
            w as f64 / rw as f64,
        )
    } else {
        (Cow::Borrowed(img), 1.0, 1.0)
    }
}

/// 单输入模型推理，返回输出数据与维度。
fn run_single(
    session: &mut Session,
    data: Vec<f32>,
    shape: [usize; 4],
) -> Result<(Vec<f32>, Vec<usize>)> {
    let tensor = Tensor::from_array((shape, data)).map_err(|e| anyhow!("创建输入张量失败: {e}"))?;
    let outputs = session
        .run(ort::inputs![tensor])
        .map_err(|e| anyhow!("推理失败: {e}"))?;
    let (out_shape, values) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|e| anyhow!("提取输出失败: {e}"))?;
    let dims: Vec<usize> = out_shape.iter().map(|&d| d as usize).collect();
    Ok((values.to_vec(), dims))
}

fn run_det(m: &ModelSet, work: &RgbImage) -> Result<Vec<[[f64; 2]; 4]>> {
    let h = ((work.h as f64 / 32.0).round() as usize * 32).max(32);
    let w = ((work.w as f64 / 32.0).round() as usize * 32).max(32);
    let chw = to_chw(&resize(work, w, h));

    let (values, dims) = {
        let mut det = lock(&m.det);
        run_single(&mut det, chw, [1, 3, h, w])?
    };
    if dims.len() < 2 {
        bail!("det 输出维度异常 {dims:?}");
    }
    let ph = dims[dims.len() - 2];
    let pw = dims[dims.len() - 1];
    if ph == 0 || pw == 0 {
        bail!("det 输出尺寸为 0");
    }
    let boxes = db_post_process(&values, ph, pw, work.w, work.h);
    Ok(filter_boxes(boxes, work.w, work.h))
}

fn run_cls(m: &ModelSet, crop: &RgbImage) -> bool {
    let chw = resize_norm(crop, 48, 192);
    let result = {
        let mut cls = lock(&m.cls);
        run_single(&mut cls, chw, [1, 3, 48, 192])
    };
    let Ok((values, _)) = result else {
        return false;
    };
    if values.len() < 2 {
        return false;
    }
    let mut best = values[0];
    let mut best_idx = 0usize;
    for (i, &v) in values[..2].iter().enumerate() {
        if v > best {
            best = v;
            best_idx = i;
        }
    }
    best_idx == 1 && best as f64 > 0.9
}

/// CTC greedy：argmax、去连续重复、去 blank(0)，置信度为选中概率均值。
fn ctc_decode(preds: &[f32], t: usize, c: usize, chars: &[String]) -> (String, f64) {
    let mut text = String::new();
    let mut sum = 0.0;
    let mut cnt = 0usize;
    let mut last: i64 = -1;
    for step in 0..t {
        let base = step * c;
        if base + c > preds.len() {
            break;
        }
        let mut best = f32::NEG_INFINITY;
        let mut best_idx = 0usize;
        for i in 0..c {
            let v = preds[base + i];
            if v > best {
                best = v;
                best_idx = i;
            }
        }
        if best_idx != 0 && best_idx as i64 != last {
            if let Some(ch) = chars.get(best_idx) {
                text.push_str(ch);
                sum += best as f64;
                cnt += 1;
            }
        }
        last = best_idx as i64;
    }
    let score = if cnt > 0 { sum / cnt as f64 } else { 0.0 };
    (text, score)
}

fn run_rec(session: &mut Session, chars: &[String], crop: &RgbImage) -> Result<(String, f64)> {
    if crop.h < 1 {
        return Ok((String::new(), 0.0));
    }
    let mut w = (48.0 * crop.w as f64 / crop.h as f64).ceil() as usize;
    if w < 320 {
        w = 320;
    }
    let chw = resize_norm(crop, 48, w);
    let (values, dims) = run_single(session, chw, [1, 3, 48, w])?;
    if dims.len() < 3 {
        bail!("rec 输出维度异常 {dims:?}");
    }
    let t = dims[dims.len() - 2];
    let c = dims[dims.len() - 1];
    Ok(ctc_decode(&values, t, c, chars))
}

/// 执行全部文本行的 rec 识别；多个 rec 会话按行轮转并发执行。
fn run_rec_batch(m: &ModelSet, jobs: &[Job]) -> Vec<Option<(String, f64)>> {
    let mut out: Vec<Option<(String, f64)>> = vec![None; jobs.len()];
    if jobs.is_empty() {
        return out;
    }

    let sessions: Vec<Session> = {
        let mut pool = lock(&m.recs);
        std::mem::take(&mut *pool)
    };
    if sessions.is_empty() {
        return out;
    }
    let workers = sessions.len();
    let chars = m.chars.clone();

    let results: Vec<(Session, Vec<(usize, String, f64)>)> = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        for (worker_idx, mut session) in sessions.into_iter().enumerate() {
            let chars = chars.clone();
            handles.push(scope.spawn(move || {
                let mut produced = Vec::new();
                let mut i = worker_idx;
                while i < jobs.len() {
                    if let Ok((text, score)) = run_rec(&mut session, &chars, &jobs[i].crop) {
                        if score >= 0.5 && !text.is_empty() {
                            produced.push((i, text, score));
                        }
                    }
                    i += workers;
                }
                (session, produced)
            }));
        }
        handles
            .into_iter()
            .map(|h| match h.join() {
                Ok(v) => v,
                Err(_) => panic!("rec 识别线程异常退出"),
            })
            .collect()
    });

    let mut back = Vec::with_capacity(results.len());
    for (session, produced) in results {
        back.push(session);
        for (i, text, score) in produced {
            out[i] = Some((text, score));
        }
    }
    *lock(&m.recs) = back;
    out
}

fn recognize_with(m: &ModelSet, img: &RgbImage, opts: &OcrOptions) -> Result<Vec<TextBlock>> {
    let (work, ratio_h, ratio_w) = preprocess(img, opts.max_side_len);
    let work = work.as_ref();
    let boxes = run_det(m, work)?;

    let mut jobs = Vec::with_capacity(boxes.len());
    for quad in boxes {
        let mut crop = warp_crop(work, &quad);
        if opts.angle && run_cls(m, &crop) {
            crop = rotate180(&crop);
        }
        jobs.push(Job { quad, crop });
    }

    let recs = run_rec_batch(m, &jobs);

    let mut blocks = Vec::with_capacity(jobs.len());
    for (i, result) in recs.into_iter().enumerate() {
        let Some((text, score)) = result else {
            continue;
        };
        let mut quad = [[0f64; 2]; 4];
        for (k, p) in jobs[i].quad.iter().enumerate() {
            quad[k][0] = (p[0] * ratio_w).clamp(0.0, img.w as f64);
            quad[k][1] = (p[1] * ratio_h).clamp(0.0, img.h as f64);
        }
        blocks.push(TextBlock::new(quad, score, text));
    }
    Ok(blocks)
}

/// 计算平均置信度、应用忽略区域、排版解析并按 dataFormat 输出。
fn finish_ocr(mut blocks: Vec<TextBlock>, opts: &OcrOptions) -> OcrResult {
    let avg = if blocks.is_empty() {
        0.0
    } else {
        blocks.iter().map(|b| b.score).sum::<f64>() / blocks.len() as f64
    };
    blocks = apply_ignore_area(blocks, &opts.ignore_area);
    blocks = apply_parser(blocks, &opts.parser);
    if blocks.is_empty() {
        return OcrResult::empty();
    }
    if opts.data_format == "text" {
        OcrResult {
            code: 100,
            data: OcrData::Text(get_data_text(&blocks)),
            score: Some(avg),
        }
    } else {
        OcrResult {
            code: 100,
            data: OcrData::Blocks(blocks),
            score: Some(avg),
        }
    }
}

/// 解析并初始化 ONNX Runtime 动态库，返回全局 Environment。
/// 查找顺序：ocr.ortLib -> ORT_DYLIB_PATH -> 可执行文件目录 -> 引擎目录 -> 当前目录 -> 系统路径。
pub fn init_ort_environment(ort_lib: Option<&str>, engine_dir: &Path) -> Result<Environment> {
    let lib_name = format!(
        "{}onnxruntime{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let cwd = std::env::current_dir().ok();

    // 用户显式指定的路径一律转成绝对路径：Windows 会自带一个不含 OrtGetApiBase 的
    // System32\onnxruntime.dll，相对路径会被 DLL 搜索顺序错误命中它。
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut push_user = |p: &str| {
        let p = PathBuf::from(p.trim());
        if p.is_absolute() {
            candidates.push(p);
            return;
        }
        if let Some(dir) = &cwd {
            candidates.push(dir.join(&p));
        }
        if let Some(dir) = &exe_dir {
            candidates.push(dir.join(&p));
        }
        candidates.push(p);
    };
    if let Some(p) = ort_lib {
        if !p.trim().is_empty() {
            push_user(p);
        }
    }
    if let Ok(p) = std::env::var("ORT_DYLIB_PATH") {
        if !p.trim().is_empty() {
            push_user(&p);
        }
    }
    drop(push_user);
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(&lib_name));
            candidates.push(dir.join("lib").join(&lib_name));
            candidates.push(dir.join("libs").join(&lib_name));
            candidates.push(dir.join("runtime").join(&lib_name));
        }
    }
    candidates.push(engine_dir.join(&lib_name));
    candidates.push(engine_dir.join("lib").join(&lib_name));
    candidates.push(engine_dir.join("runtime").join(&lib_name));
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(&lib_name));
        candidates.push(cwd.join("lib").join(&lib_name));
        candidates.push(cwd.join("runtime").join(&lib_name));
    }
    #[cfg(unix)]
    {
        for dir in ["/usr/lib", "/usr/local/lib", "/opt/homebrew/lib", "/opt/local/lib"] {
            candidates.push(PathBuf::from(dir).join(&lib_name));
        }
    }

    for path in &candidates {
        if !path.is_file() {
            continue;
        }
        let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        let env = ort::init_from(&abs)
            .map_err(|e| anyhow!("加载 ONNX Runtime 动态库失败 {}: {e}", abs.display()))?
            .with_name("umiocr-http-rs")
            .build()
            .map_err(|e| anyhow!("初始化 ONNX Runtime 失败: {e}"))?;
        tracing::info!("ONNX Runtime 动态库: {}", abs.display());
        return Ok(env);
    }

    Err(anyhow!(
        "未找到 ONNX Runtime 动态库 {lib_name}。请将 {lib_name} 放到可执行文件同目录，\
         或在 configs/application.yaml 中设置 ocr.ortLib，或设置环境变量 ORT_DYLIB_PATH。\
         （需要 ONNX Runtime >= 1.17，推荐 1.22+）"
    ))
}
