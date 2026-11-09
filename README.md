# Umi-OCR HTTP API（Rust 实现）

参考 [umiocr-http-go](../umiocr-http-go) 改写而成的 Rust 版本：复用 Umi-OCR 的 **RapidOCR 模型文件**，在 Windows / Linux / macOS 上以纯 Rust 方式完成 PP-OCR（检测 + 方向分类 + 识别）推理，并提供与 Umi-OCR 兼容的 HTTP API。

- 推理**不手写** ONNX Runtime 绑定，也不实现算子：通过 [`ort`](https://crates.io/crates/ort) 的 `load-dynamic` 在运行时加载各平台官方动态库（`onnxruntime.dll` / `libonnxruntime.so` / `libonnxruntime.dylib`）。
- 不需要 `RapidOCR-json.exe` 子进程，模型直接加载进进程。
- 排版解析、忽略区域、二维码生成/识别、错误码均与 Go 版 / Umi-OCR 对齐。

## 目录结构

```
umiocr-http-rs/
├── Cargo.toml
├── configs/application.yaml    # 服务与 OCR 配置
├── src/
│   ├── main.rs                 # 启动、预热、优雅退出
│   ├── config.rs               # YAML 配置结构
│   ├── http.rs                 # axum 路由与各接口实现
│   └── ocr/
│       ├── engine.rs           # ORT 动态库加载、会话管理、推理主流程
│       ├── det.rs              # DB 后处理（连通域、凸包、最小外接矩形、unclip）
│       ├── imgproc.rs          # 解码/缩放/CHW/RGB、四边形裁剪、旋转
│       ├── tbpu.rs             # 排版解析（parsers）与忽略区域
│       ├── options.rs          # 请求 options 解析、get_options 返回
│       └── types.rs            # 请求/响应结构
└── web/index.html              # 简易测试页
```

## 环境要求

| 依赖 | 要求 |
| --- | --- |
| Rust | 1.75+（edition 2021） |
| ONNX Runtime | **>= 1.17，推荐 1.22+**（`ORT_API_VERSION=17`） |
| 模型 | Umi-OCR `RapidOCR-json` 插件里的 `models` 目录（PP-OCR det/cls/rec + keys） |

获取 ONNX Runtime（CPU 版，与官网发布同源）：

- GitHub Releases：`onnxruntime-win-x64-<版本>.zip` / `onnxruntime-linux-x64-<版本>.tgz` / `onnxruntime-osx-universal2-<版本>.tgz`
- 或从 PyPI 的 `onnxruntime` wheel 中提取：`onnxruntime/capi/onnxruntime.dll`（wheel 改名为 `.zip` 后解压即可）

## 构建

### Windows（推荐 MSVC 工具链）

```powershell
rustup default stable-x86_64-pc-windows-msvc
cargo build --release
```

若使用 `x86_64-pc-windows-gnu` 工具链：`libloading 0.9` 通过 raw-dylib 生成导入库，需要 `dlltool.exe` 与 `as.exe`（MSYS2 / mingw-w64 的 binutils）。把它们放进
`%USERPROFILE%\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained\`
或加入 `PATH` 即可。

### Linux

```bash
cargo build --release
```

### macOS

```bash
cargo build --release
```

产物：`target/release/umiocr-http-rs`（Windows 为 `umiocr-http-rs.exe`）。

## 打包

`scripts/` 下提供了两个平台的打包脚本：编译 → 组装 `dist/umiocr-http-rs-<版本>-<目标三元组>/`（可执行文件、`configs/`、`web/`、`README.md`、可选的 `onnxruntime` 动态库与 `models/`）→ 压缩。

### Windows

```powershell
# 编译 + 打包，自动附带项目根目录下的 onnxruntime.dll / models（若存在）
powershell -ExecutionPolicy Bypass -File scripts\package.ps1

# 指定动态库与模型目录
powershell -ExecutionPolicy Bypass -File scripts\package.ps1 `
    -OrtLib D:\ort\onnxruntime.dll -ModelsDir D:\Umi-OCR\models

# 只重新打包（跳过编译）
powershell -ExecutionPolicy Bypass -File scripts\package.ps1 -SkipBuild
```

产物：`dist\umiocr-http-rs-<版本>-<三元组>.zip`

### Linux / macOS

```bash
chmod +x scripts/package.sh
./scripts/package.sh                                    # 编译 + 打包
./scripts/package.sh --ort-lib /usr/lib/libonnxruntime.so --models /data/models
./scripts/package.sh --skip-build --no-archive
```

产物：`dist/umiocr-http-rs-<版本>-<三元组>.tar.gz`

脚本参数说明：

| 参数（PowerShell / Bash） | 说明 |
| --- | --- |
| `-OrtLib` / `--ort-lib` | 指定要随包分发的 ORT 动态库 |
| `-ModelsDir` / `--models` | 指定要随包分发的模型目录 |
| `-SkipBuild` / `--skip-build` | 跳过 `cargo build --release` |
| `-NoZip` / `--no-archive` | 只保留目录，不生成压缩包 |

两个脚本都会读取 `CARGO_TARGET_DIR`（若设置），并优先从该目录取构建产物。

### GitHub Actions

[.github/workflows/release.yml](.github/workflows/release.yml) 会在 GitHub 上自动打包三端制品（windows-x64 / linux-x64 / macos-arm64）：

- push `v*` 标签：构建三端制品并创建（或更新）GitHub Release
- 手动触发 `workflow_dispatch`：只产出 Actions Artifact

制品包含：可执行文件、`configs/`、`web/`、`README.md`、`THIRD_PARTY_NOTICES.md`（第三方组件许可证说明）、从官方 Releases 下载的 ONNX Runtime 动态库，以及从 `RapidOCR-json` 仓库 `models.7z` 解压出的 `models/`（PP-OCR det/cls/rec + 各语言字典），解包即可直接运行。ONNX Runtime 版本由 workflow 顶部的 `ORT_VERSION` 控制，模型来源由 `MODELS_URL` 控制。

`models.7z` 由仓库自带的纯 Rust 解压工具 [tools/unpack7z](tools/unpack7z) 解压，**不依赖系统安装 7-Zip**（Windows / Linux / macOS 通用）。本地也可单独使用：

```bash
cargo build --release -p unpack7z
./target/release/unpack7z models.7z ./models
```

## 部署与运行

运行目录建议布局（可执行文件、动态库、配置、模型放在一起）：

```
运行目录/
├── umiocr-http-rs.exe          # 或 umiocr-http-rs
├── onnxruntime.dll             # Linux: libonnxruntime.so / macOS: libonnxruntime.dylib
├── configs/application.yaml
├── models/
│   ├── configs.txt
│   ├── det.onnx
│   ├── cls.onnx
│   ├── rec.onnx
│   └── keys.txt
└── web/index.html
```

```bash
./umiocr-http-rs
# 日志：Umi-OCR HTTP API 已启动：http://127.0.0.1:1224
```

配置文件固定从**当前工作目录**的 `configs/application.yaml` 读取；缺失或解析失败会回退默认值并打印 warning。

### ONNX Runtime 动态库查找顺序

1. `ocr.ortLib`（配置项，可为相对路径；相对路径按当前目录、可执行文件目录展开）
2. 环境变量 `ORT_DYLIB_PATH`
3. 可执行文件同目录 → `lib/` → `libs/` → `runtime/`
4. `ocr.engineDir` 及其 `lib/`、`runtime/`
5. 当前工作目录及其 `lib/`、`runtime/`
6. Linux/macOS：`/usr/lib`、`/usr/local/lib`、`/opt/homebrew/lib`、`/opt/local/lib`

开发时把动态库放到项目的 `runtime/` 目录即可，无需改配置。

命中的路径会被转换成绝对路径再 `LoadLibrary`。这一点在 Windows 上很重要：系统自带的 `C:\Windows\System32\onnxruntime.dll`（Windows ML 组件）**不含** `OrtGetApiBase`，若用裸文件名加载会被它抢先命中并报
`does not export OrtGetApiBase`；本程序通过绝对路径规避。

### 模型文件准备

直接使用 Umi-OCR 的 RapidOCR-json 插件目录即可，例如
`.../UmiOCR-data/plugins/win7_x64_RapidOCR-json/models/`。

`models/configs.txt` 每 5 行为一组、组间用空行分隔：

```
简体中文
det.onnx
cls.onnx
rec.onnx
keys.txt

English
det.onnx
cls.onnx
en_rec.onnx
en_keys.txt
```

- 第 1 行：语言/模型库名称（即 API 中 `ocr.language` 的取值）
- 第 2~5 行：det 模型、cls 模型、rec 模型、识别字典（相对 `models` 目录）
- 字典首行自动视为 `blank`，末尾自动补空格

想新增语言，把对应模型与字典放进来、在 `configs.txt` 末尾追加一组即可，重启生效。

## 配置项

`configs/application.yaml`：

```yaml
server:
  port: 1224          # 监听端口，绑定 0.0.0.0
  webDir: "./web"     # 测试页目录（/index.html）

ocr:
  engineDir: "."      # 模型根目录（其下含 models 子目录）
  models: "models"    # 模型子目录名
  numThread: 4        # 每个 ONNX 会话的线程数
  recPool: 0          # rec 会话池大小，0 = 按 CPU 核心数自动（上限 4）
  ortLib: ""          # 显式指定 ORT 动态库路径，留空则自动查找
```

`recPool` 决定并发识别能力：多个请求会各自取一份 rec 会话，避免互相阻塞。

## HTTP API

服务默认监听 `0.0.0.0:1224`，并已开启 CORS（Any）。

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/`、`/umiocr` | 返回 `Umi-OCR (Rust HTTP API) v2.1.2` |
| GET | `/index.html` | 测试页面（来自 `server.webDir`） |
| POST | `/argv` | 仅本机调用，返回 `{"code":100,"data":"ok"}` |
| GET | `/api/ocr/get_options` | 获取可选语言与各 options 默认值 |
| POST | `/api/ocr` | OCR 识别 |
| POST | `/api/qrcode` | 二维码识别 / 生成 |

### POST /api/ocr

```json
{
  "base64": "<图片 base64，可带 data:image/png;base64, 前缀>",
  "options": {
    "ocr.language": "简体中文",
    "ocr.angle": true,
    "ocr.maxSideLen": 1024,
    "tbpu.parser": "multi_para",
    "tbpu.ignoreArea": [[[10, 10], [200, 60]]],
    "data.format": "dict"
  }
}
```

`options` 可省略或部分省略，缺省值：语言取 `configs.txt` 第一组、`angle=false`、`maxSideLen=1024`、`parser=multi_para`、`data.format=dict`。

响应：

```json
{
  "code": 100,
  "data": [
    {"box": [[41,42],[585,42],[585,84],[41,84]], "score": 0.976, "text": "第一行", "end": "\n"}
  ],
  "score": 0.972
}
```

`data.format=text` 时 `data` 为字符串（按 parser 处理后的纯文本）。

#### options 说明

| 键 | 类型 | 说明 |
| --- | --- | --- |
| `ocr.language` | string | 语言/模型库名，取自 `configs.txt` 每组首行 |
| `ocr.angle` | bool | 是否启用方向分类（0°/180° 纠正） |
| `ocr.maxSideLen` | int | 检测前长边缩放上限，`0` 表示不缩放 |
| `tbpu.parser` | string | 排版解析方式，见下表 |
| `tbpu.ignoreArea` | array | 忽略区域，`[[[x1,y1],[x2,y2]], ...]` |
| `data.format` | string | `dict`（默认，返回文字块数组）或 `text` |

#### 排版解析 `tbpu.parser`

| 取值 | 含义 |
| --- | --- |
| `multi_para` | 多栏-按自然段换行（默认） |
| `multi_line` | 多栏-总是换行 |
| `multi_none` | 多栏-无换行 |
| `single_para` | 单栏-按自然段换行 |
| `single_line` | 单栏-总是换行 |
| `single_none` | 单栏-无换行 |
| `single_code` | 单栏-保留缩进（代码/表格） |
| `none` | 不做处理 |

`tbpu.ignoreArea` 用于屏蔽水印等固定区域，坐标为**相对原图**的像素矩形，落在框内的文字块会被丢弃。

### POST /api/qrcode

识别（传 `base64`）：

```json
{"base64": "<二维码图片 base64>"}
```

```json
{"code": 100, "data": [{"text": "hello", "format": "QRCode", "orientation": 0}]}
```

生成（传 `text`，可附 `options`）：

```json
{"text": "hello", "options": {"ec_level": 1, "w": 300}}
```

`ec_level`：`0=L / 1=M / 2=Q / 3=H`（默认 M）；`w` 期望边长像素（默认约 4 像素/模块）。返回 JPEG 的 base64：

```json
{"code": 100, "data": "/9j/4AAQSkZJRg..."}
```

### 错误码

| code | 含义 |
| --- | --- |
| 100 | 成功 |
| 101 | 识别成功但结果为空 |
| 300 | 识别失败（如未知语言/模型库、图片解码失败等） |
| 800 | 请求无法解析为 JSON |
| 801 | 请求为空 |
| 802 | 缺少 `base64` 字段或其类型错误 |
| 803 | `options` 字段不是字典 |
| 804 | `options` 解释失败（如 `tbpu.ignoreArea` 格式错误） |

### 调用示例

```bash
curl -s http://127.0.0.1:1224/api/ocr/get_options

curl -s -X POST http://127.0.0.1:1224/api/ocr \
  -H "Content-Type: application/json" \
  -d "{\"base64\":\"$(base64 -w0 test.png)\",\"options\":{\"data.format\":\"text\"}}"

curl -s -X POST http://127.0.0.1:1224/api/qrcode \
  -H "Content-Type: application/json" \
  -d '{"text":"hello"}'
```

浏览器打开 `http://127.0.0.1:1224/index.html` 可使用内置测试页。

请求体上限 128 MB。

## 与 Go 版的主要差异

| 项 | Go 版 | Rust 版 |
| --- | --- | --- |
| 推理方式 | 纯 Go 实现 ONNX 算子 / 回退 `RapidOCR-json.exe` | `ort` 加载平台官方 ORT 动态库，直接跑 `.onnx` |
| 外部依赖 | RapidOCR-json 插件目录 + 模型 | ORT 动态库 + 模型目录 |
| 运行形态 | 单可执行文件 | 单可执行文件（动态库随目录分发） |
| API / 错误码 / parser | — | 完全对齐 |

## 常见问题

**启动报 `does not export OrtGetApiBase`**
说明加载到的不是真正的 ONNX Runtime。检查是否被 `C:\Windows\System32\onnxruntime.dll` 抢先命中，或版本低于 1.17。用 `ocr.ortLib` / `ORT_DYLIB_PATH` 指定绝对路径。

**启动报 `模型配置为空` 或读取 `configs.txt` 失败**
`ocr.engineDir` + `ocr.models` 拼出的目录下必须存在 `configs.txt`，且文件内容格式正确（每 5 行一组、组间空行）。

**Linux 上找不到 `libonnxruntime.so`**
放到可执行文件同目录，或设置 `ORT_DYLIB_PATH=/path/to/libonnxruntime.so`，或写入 `/usr/local/lib` 后 `ldconfig`。

**识别结果把中文识别成别的语言**
`ocr.language` 必须与 `configs.txt` 中某一组首行完全一致；不传则用第一组。

## 许可证

MIT
