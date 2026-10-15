#!/usr/bin/env bash
# Linux 交叉编译并组装 Windows x64 ZIP。
# 依赖：cargo-xwin、x86_64-pc-windows-msvc Rust target、Python 3.11+。
# 用法：bash scripts/package-windows.sh
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET="x86_64-pc-windows-msvc"

echo "==> 交叉编译（release）"
cargo xwin build --locked --release --target "${TARGET}"

echo "==> 组装 Windows 分发包"
python3 scripts/package-windows.py --binary "target/${TARGET}/release/rust-mhxy.exe"
