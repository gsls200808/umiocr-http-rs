# 第三方组件与许可证说明

本制品（`umiocr-http-rs`）在运行时或分发时包含下列第三方组件。各组件的版权归其作者所有，使用时请遵守对应许可证。

## 1. ONNX Runtime（推理运行时，动态库）

| 项目 | 说明 |
| --- | --- |
| 组件 | ONNX Runtime（CPU 版） |
| 版本 | 1.22.0（随制品分发的 `onnxruntime.dll` / `libonnxruntime.so` / `libonnxruntime.dylib`） |
| 许可证 | MIT |
| 版权 | Copyright (c) Microsoft Corporation |
| 来源 | https://github.com/microsoft/onnxruntime |
| 许可文本 | https://github.com/microsoft/onnxruntime/blob/main/LICENSE |

程序通过运行时动态加载该库（`ort` crate 的 `load-dynamic` 特性），未静态链接、未修改其代码。

## 2. OCR 识别模型（PP-OCR / PaddleOCR）

| 项目 | 说明 |
| --- | --- |
| 组件 | PP-OCR det / cls / rec 模型（ONNX）及字典文件（`models/` 目录） |
| 许可证 | Apache License 2.0 |
| 版权 | Copyright (c) PaddlePaddle Authors（百度 PaddleOCR 团队） |
| 来源（模型） | https://github.com/hiroi-sora/RapidOCR-json 仓库中的 `models.7z` |
| 上游 | https://github.com/PaddlePaddle/PaddleOCR |
| 许可文本 | https://www.apache.org/licenses/LICENSE-2.0 |

模型文件保持原样分发，未做修改。

## 3. 兼容与参考实现

| 组件 | 许可证 | 说明 |
| --- | --- | --- |
| Umi-OCR（https://github.com/hiroi-sora/Umi-OCR） | MIT | HTTP API 协议、排版解析规则、配置项命名与之兼容 |
| RapidOCR-json（https://github.com/hiroi-sora/RapidOCR-json） | MIT | 模型目录布局（`models/configs.txt`）与后处理参数参考 |
| RapidOCR（https://github.com/RapidAI/RapidOCR） | Apache-2.0 | 模型转换与推理参数参考 |

本项目为独立实现，未包含上述项目的源代码。

## 4. Rust 依赖（直接依赖）

| Crate | 版本 | 许可证 |
| --- | --- | --- |
| anyhow | 1.0 | MIT OR Apache-2.0 |
| axum | 0.8 | MIT |
| base64 | 0.22 | MIT OR Apache-2.0 |
| image | 0.25 | MIT OR Apache-2.0 |
| ort | 2.0.0-rc.15 | MIT OR Apache-2.0 |
| qrcode | 0.14 | MIT OR Apache-2.0 |
| rqrr | 0.11 | (MIT OR Apache-2.0) AND ISC |
| serde / serde_json | 1.0 | MIT OR Apache-2.0 |
| serde_yaml | 0.9 | MIT OR Apache-2.0 |
| tokio | 1.x | MIT |
| tower-http | 0.6 | MIT |
| tracing / tracing-subscriber | 0.1 / 0.3 | MIT |

## 5. 传递依赖

完整的第三方 crate 依赖树约 117 个包，许可证均为宽松许可：MIT、Apache-2.0、BSD-3-Clause、Zlib、ISC、Unlicense 或它们的组合，不包含 GPL / LGPL / AGPL / MPL 等要求开源衍生作品的许可证。

如需逐个核对，可在源码仓库中执行：

```bash
cargo metadata --format-version 1 | jq -r '.packages[] | "\(.name) \(.version) \(.license)"' | sort -u
```

各 crate 的许可证全文可从 https://crates.io 或对应仓库获取；本地构建时也已随 crate 源码缓存在 `~/.cargo/registry/src/` 下的 `LICENSE*` 文件中。

## 6. 构建与打包工具（不随制品分发）

| 组件 | 许可证 | 用途 |
| --- | --- | --- |
| sevenz-rust（https://github.com/dyz1990/sevenz-rust） | Apache-2.0 | 仓库内 `tools/unpack7z` 使用，用于在打包时解压 `models.7z`，不进入制品 |
