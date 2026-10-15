#!/usr/bin/env bash
# 在 Linux 上交叉编译出 Windows 测试包（zip）。
#
# 依赖：
#   - rustup target add x86_64-pc-windows-msvc
#   - cargo install cargo-xwin --locked
#   - 7z / curl / zip
#
# 用法：bash scripts/package-windows.sh
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET="x86_64-pc-windows-msvc"
STAGE="dist/rust-mhxy-windows"
ZIP_NAME="rust-mhxy-windows.zip"
RUNTIME_TMP="dist/.runtime"
MODELS_DIR="${HOME}/.oar"

echo "==> 交叉编译（release）"
cargo xwin build --locked --release --target "${TARGET}"

# 检查实际产物，避免误发仍会弹出命令窗口的控制台程序。
python3 - "target/${TARGET}/release/rust-mhxy.exe" <<'PY'
import pathlib, struct, sys
exe = pathlib.Path(sys.argv[1])
data = exe.read_bytes()
pe_offset = struct.unpack_from("<I", data, 0x3c)[0]
if data[pe_offset:pe_offset + 4] != b"PE\0\0":
    raise SystemExit("错误：Windows EXE 的 PE 文件头无效")
subsystem = struct.unpack_from("<H", data, pe_offset + 24 + 68)[0]
if subsystem != 2:
    raise SystemExit(f"错误：Windows EXE 不是 GUI 子系统（{subsystem}），启动仍会弹出命令窗口")
print("==> EXE 已验证为 Windows GUI 子系统（无命令窗口）")
PY

echo "==> 准备 VC++ 运行库（官方 msvc-runtime wheel）"
mkdir -p "${RUNTIME_TMP}"
if [ ! -f "${RUNTIME_TMP}/extracted/.ok" ]; then
  rm -rf "${RUNTIME_TMP}/extracted"
  mkdir -p "${RUNTIME_TMP}/extracted"
  WHEEL_URL=$(curl -s https://pypi.org/pypi/msvc-runtime/json | python3 -c \
    "import sys,json;d=json.load(sys.stdin);print(next(f['url'] for f in d['releases'][d['info']['version']] if 'win_amd64' in f['filename'] and f['filename'].endswith('.whl')))")
  echo "    ${WHEEL_URL}"
  curl -sL -o "${RUNTIME_TMP}/msvc.whl" "${WHEEL_URL}"
  7z x -y "${RUNTIME_TMP}/msvc.whl" -o"${RUNTIME_TMP}/extracted" >/dev/null
  touch "${RUNTIME_TMP}/extracted/.ok"
fi

echo "==> 组装 ${STAGE}"
rm -rf "${STAGE}" && mkdir -p "${STAGE}/assets/models"
cp "target/${TARGET}/release/rust-mhxy.exe" "${STAGE}/"
cp "$(ls ~/.cache/ort.pyke.io/dfbin/${TARGET}/*/DirectML.dll | head -1)" "${STAGE}/"
for dll in msvcp140.dll msvcp140_1.dll msvcp140_2.dll msvcp140_atomic_wait.dll \
           msvcp140_codecvt_ids.dll vcruntime140.dll vcruntime140_1.dll \
           vcruntime140_threads.dll concrt140.dll; do
  src=$(find "${RUNTIME_TMP}/extracted" -iname "${dll}" | head -1 || true)
  [ -n "${src}" ] && cp "${src}" "${STAGE}/"
done
cp config.example.json "${STAGE}/"
cp assets/maps.json "${STAGE}/assets/maps.json"
cp -r assets/maps "${STAGE}/assets/maps"
if [ -f "${MODELS_DIR}/pp-ocrv5_mobile_det.onnx" ]; then
  cp "${MODELS_DIR}/pp-ocrv5_mobile_det.onnx" \
     "${MODELS_DIR}/pp-ocrv5_mobile_rec.onnx" \
     "${MODELS_DIR}/ppocrv5_dict.txt" "${STAGE}/assets/models/"
else
  echo "    提示：未找到 ${MODELS_DIR} 下的本地 OCR 模型，包内不内置（首次使用会联网下载）"
fi

python3 - "$STAGE" <<'PY'
import sys, pathlib
stage = pathlib.Path(sys.argv[1])
text = """rust-mhxy - 梦幻西游抓鬼/鬼王任务坐标辅助（Windows 测试版）
============================================================

一、运行
  1. 解压整个文件夹到任意目录（assets 目录要一起）。
  2. 双击 rust-mhxy.exe 运行，启动时不会弹出命令窗口。

二、说明
  - 本地 PP-OCR 模型已随包放在 assets/models，离线可用。
  - 远端 PP-OCR：选「远端 PP-OCR」，填服务地址与 Token，可点「测试连接」。
  - 游戏窗口会自动扫描并选中（优先标题含「梦幻西游 / MHXY」），也可手动选择。
  - 左侧栏底部日志常开，分「简单 / 详细」。
  - 「设置识别区域」按钮会打开独立窗口，可点击截图取点取色（图色门控）。
  - 使用顶部按钮开始或暂停识别；任务状态显示在任务卡片标题右侧。
  - 配置保存到同目录 config.json，可参考 config.example.json。

三、运行库
  - 已自带 VC++ 运行库与 DirectML.dll，需要 Windows 10/11 64 位。
"""
(stage / "使用说明.txt").write_text(text, encoding="utf-8-sig")
PY

echo "==> 打包 ${ZIP_NAME}"
( cd dist && rm -f "${ZIP_NAME}" && zip -rq "${ZIP_NAME}" "$(basename "${STAGE}")" )
ls -la "dist/${ZIP_NAME}"
