#!/usr/bin/env bash
# 构建并打包 umiocr-http-rs 的 Linux / macOS 发行包。
#
# 用法:
#   ./scripts/package.sh                        # 编译 + 打包，自动附带 models/ 与 onnxruntime 库（若在项目根目录）
#   ./scripts/package.sh --ort-lib /usr/lib/libonnxruntime.so --models /data/models
#   ./scripts/package.sh --skip-build --no-archive
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ORT_LIB=""
MODELS_DIR=""
SKIP_BUILD=0
NO_ARCHIVE=0

usage() {
    sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --ort-lib)   ORT_LIB="${2:?--ort-lib 需要路径参数}"; shift 2 ;;
        --models)    MODELS_DIR="${2:?--models 需要目录参数}"; shift 2 ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        --no-archive) NO_ARCHIVE=1; shift ;;
        -h|--help)   usage; exit 0 ;;
        *) echo "未知参数: $1" >&2; usage; exit 1 ;;
    esac
done

find_tool() {
    local name="$1" fallback="$HOME/.cargo/bin/$1"
    if command -v "$name" >/dev/null 2>&1; then
        command -v "$name"
    elif [[ -x "$fallback" ]]; then
        echo "$fallback"
    else
        echo "未找到 $name，请先安装 Rust 工具链（https://rustup.rs）" >&2
        exit 1
    fi
}

CARGO="$(find_tool cargo)"
RUSTC="$(find_tool rustc)"

VERSION="$(awk -F'"' '/^version[[:space:]]*=/{print $2; exit}' Cargo.toml)"
HOST_TRIPLE="$("$RUSTC" -vV | awk '/^host:/{print $2}')"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
BIN="$TARGET_DIR/release/umiocr-http-rs"

echo "==> 项目版本: $VERSION ($HOST_TRIPLE)"

if [[ $SKIP_BUILD -eq 0 ]]; then
    echo "==> cargo build --release"
    "$CARGO" build --release
else
    echo "==> 跳过编译（--skip-build）"
fi

if [[ ! -f "$BIN" ]]; then
    echo "未找到构建产物 $BIN，请去掉 --skip-build 重新打包" >&2
    exit 1
fi

PKG_NAME="umiocr-http-rs-$VERSION-$HOST_TRIPLE"
DIST_ROOT="$ROOT/dist"
OUT_DIR="$DIST_ROOT/$PKG_NAME"

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR/configs"

echo "==> 组装 $PKG_NAME"
cp "$BIN" "$OUT_DIR/"
cp configs/application.yaml "$OUT_DIR/configs/"
cp -R web "$OUT_DIR/web"
cp README.md THIRD_PARTY_NOTICES.md "$OUT_DIR/"

# ONNX Runtime 动态库：显式指定优先，其次项目根目录 / runtime 目录
if [[ -z "$ORT_LIB" ]]; then
    for candidate in "$ROOT/libonnxruntime.so" "$ROOT/libonnxruntime.dylib" \
                     "$ROOT/runtime/libonnxruntime.so" "$ROOT/runtime/libonnxruntime.dylib"; do
        if [[ -f "$candidate" ]]; then ORT_LIB="$candidate"; break; fi
    done
fi
if [[ -n "$ORT_LIB" ]]; then
    if [[ ! -f "$ORT_LIB" ]]; then
        echo "ONNX Runtime 动态库不存在: $ORT_LIB" >&2
        exit 1
    fi
    cp "$ORT_LIB" "$OUT_DIR/"
    echo "    已附带 $(basename "$ORT_LIB") <- $ORT_LIB"
else
    echo "    warning: 未附带 onnxruntime 动态库，请在运行前自行放入包目录（或用 --ort-lib 指定）" >&2
fi

# 模型目录：显式指定优先，其次项目根目录下的 models
if [[ -z "$MODELS_DIR" && -d "$ROOT/models" ]]; then
    MODELS_DIR="$ROOT/models"
fi
if [[ -n "$MODELS_DIR" ]]; then
    if [[ ! -d "$MODELS_DIR" ]]; then
        echo "模型目录不存在: $MODELS_DIR" >&2
        exit 1
    fi
    cp -R "$MODELS_DIR" "$OUT_DIR/models"
    echo "    已附带 models <- $MODELS_DIR"
else
    echo "    warning: 未附带 models 目录，请在运行前自行放入包目录（或用 --models 指定）" >&2
fi

if [[ $NO_ARCHIVE -eq 0 ]]; then
    ARCHIVE="$DIST_ROOT/$PKG_NAME.tar.gz"
    rm -f "$ARCHIVE"
    echo "==> 生成压缩包"
    tar -C "$DIST_ROOT" -czf "$ARCHIVE" "$PKG_NAME"
    echo "==> 完成: $ARCHIVE ($(du -h "$ARCHIVE" | cut -f1))"
else
    echo "==> 完成: $OUT_DIR"
fi
