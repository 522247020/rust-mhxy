#!/usr/bin/env bash
# Linux 交叉编译并组装 Windows x64 ZIP。
# 依赖：cargo-xwin、x86_64-pc-windows-msvc Rust target、Python 3.11+。
# 用法：bash scripts/package-windows.sh
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET="x86_64-pc-windows-msvc"

# 用 cargo 自己报告的 target_directory 定位产物，天然跟随 CARGO_TARGET_DIR。
# 不能猜 target/：CI 会把 CARGO_TARGET_DIR 指到持久化缓存目录。
target_dir="$(cargo metadata --format-version 1 --no-deps 2>/dev/null |
  python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')" ||
  target_dir="${CARGO_TARGET_DIR:-$PWD/target}"
echo "CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-<unset>} → 产物目录：$target_dir"

echo "==> 交叉编译（release）"
cargo xwin build --locked --release --target "${TARGET}"

binary="$target_dir/${TARGET}/release/rust-mhxy.exe"
if [ ! -f "$binary" ]; then
  echo "未在 $binary 找到产物，搜索 $target_dir …" >&2
  found="$(find "$target_dir" -type f -name 'rust-mhxy.exe' 2>/dev/null | head -1 || true)"
  if [ -n "$found" ]; then
    binary="$found"
  else
    echo "打包失败：找不到 rust-mhxy.exe（target_directory=$target_dir）" >&2
    exit 1
  fi
fi
echo "EXE：$binary"

echo "==> 组装 Windows 分发包"
python3 scripts/package-windows.py --binary "$binary"
