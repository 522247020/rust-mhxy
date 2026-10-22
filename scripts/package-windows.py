#!/usr/bin/env python3
"""组装 Windows x64 分发包，供 GitHub Actions 和 Linux 交叉编译共用。"""

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import struct
import sys
import time
import tomllib
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
TARGET = "x86_64-pc-windows-msvc"
MODEL_BASE_URL = "https://github.com/GreatV/oar-ocr/releases/download/v0.3.0"
# 与 oar-ocr 0.10.0 的模型注册表一致；缓存和新下载都要校验。
MODEL_HASHES = {
    "pp-ocrv5_mobile_det.onnx": "1eb7b4f7ab657ebd1c66d5f79bca7497f29768a2e3c15e52daecbba1a8e4a039",
    "pp-ocrv5_mobile_rec.onnx": "243a0f06d826761323e9045e9b113ab2c191c3aa50565585e628300b8eda0224",
    "ppocrv5_dict.txt": "d1979e9f794c464c0d2e0b70a7fe14dd978e9dc644c0e71f14158cdf8342af1b",
}
RUNTIME_URL = (
    "https://files.pythonhosted.org/packages/21/3b/"
    "134d04268ab8e35853cd007582076429b45d60d6abb1036d159be9c50342/"
    "msvc_runtime-14.44.35112-cp312-cp312-win_amd64.whl"
)
RUNTIME_HASH = "32f9c706009e16ccc319d6947ce3bffe20e5192bee52b18cf48313f9e7bedfbe"
RUNTIME_DLLS = (
    "msvcp140.dll", "msvcp140_1.dll", "msvcp140_2.dll",
    "msvcp140_atomic_wait.dll", "msvcp140_codecvt_ids.dll",
    "vcruntime140.dll", "vcruntime140_1.dll", "vcruntime140_threads.dll",
    "concrt140.dll",
)


def sha256(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def download_verified(url: str, destination: Path, expected_hash: str) -> None:
    if destination.is_file() and sha256(destination) == expected_hash:
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_suffix(destination.suffix + ".tmp")
    for attempt in range(3):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": "rust-mhxy-packager"})
            with urllib.request.urlopen(request, timeout=60) as source, temporary.open("wb") as output:
                shutil.copyfileobj(source, output)
            if sha256(temporary) != expected_hash:
                raise ValueError(f"下载文件 SHA256 不符：{destination.name}")
            temporary.replace(destination)
            return
        except (OSError, ValueError):
            if attempt == 2:
                raise
            time.sleep(attempt + 1)
        finally:
            temporary.unlink(missing_ok=True)


def check_gui_executable(binary: Path) -> None:
    data = binary.read_bytes()
    if data[:2] != b"MZ" or len(data) < 64:
        raise ValueError("Windows EXE 的 DOS 文件头无效")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if len(data) < pe + 94 or data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("Windows EXE 的 PE 文件头无效")
    if struct.unpack_from("<H", data, pe + 4)[0] != 0x8664:
        raise ValueError("Windows EXE 不是 x86_64 架构")
    if struct.unpack_from("<H", data, pe + 24)[0] != 0x20B:
        raise ValueError("Windows EXE 不是 PE32+ 格式")
    if struct.unpack_from("<H", data, pe + 24 + 68)[0] != 2:
        raise ValueError("Windows EXE 不是 GUI 子系统，启动仍会弹出命令窗口")


def default_ort_cache() -> Path:
    if os.environ.get("ORT_CACHE_DIR"):
        return Path(os.environ["ORT_CACHE_DIR"])
    if os.name == "nt":
        return Path(os.environ["LOCALAPPDATA"]) / "ort.pyke.io"
    if os.environ.get("XDG_CACHE_HOME"):
        return Path(os.environ["XDG_CACHE_HOME"])
    return Path.home() / ".cache" / "ort.pyke.io"


def package(binary: Path, output: Path, models: Path, ort_cache: Path, version: str) -> Path:
    check_gui_executable(binary)
    print("EXE 已验证为 Windows x64 GUI 子系统（无命令窗口）", flush=True)
    directml_candidates = [binary.parent / "DirectML.dll"]
    directml_candidates.extend(sorted(
        (ort_cache / "dfbin" / TARGET).glob("*/DirectML.dll"),
        key=lambda path: path.stat().st_mtime, reverse=True,
    ))
    directml = next((path for path in directml_candidates if path.is_file()), None)
    if directml is None:
        raise FileNotFoundError(f"未找到 {ort_cache} 中的 DirectML.dll，请先完成 Windows 构建")

    runtime = output / ".runtime" / Path(RUNTIME_URL).name
    print("准备 VC++ 运行库和 PP-OCRv5 离线模型", flush=True)
    download_verified(RUNTIME_URL, runtime, RUNTIME_HASH)
    for name, digest in MODEL_HASHES.items():
        download_verified(f"{MODEL_BASE_URL}/{name}", models / name, digest)

    stage = output / f"rust-mhxy-{version}-windows-x86_64"
    if stage.exists():
        shutil.rmtree(stage)
    (stage / "assets" / "models").mkdir(parents=True)
    shutil.copy2(binary, stage / "rust-mhxy.exe")
    shutil.copy2(directml, stage / "DirectML.dll")
    with zipfile.ZipFile(runtime) as wheel:
        for name in RUNTIME_DLLS:
            member = next((item for item in wheel.namelist() if Path(item).name.lower() == name), None)
            if member is None:
                raise FileNotFoundError(f"VC++ 运行库包缺少 {name}")
            with wheel.open(member) as source, (stage / name).open("wb") as destination:
                shutil.copyfileobj(source, destination)
    shutil.copy2(ROOT / "config.example.json", stage)
    shutil.copy2(ROOT / "LICENSE", stage)
    shutil.copy2(ROOT / "assets" / "maps.json", stage / "assets")
    shutil.copytree(ROOT / "assets" / "maps", stage / "assets" / "maps", ignore=shutil.ignore_patterns(".gitkeep"))
    for name in MODEL_HASHES:
        shutil.copy2(models / name, stage / "assets" / "models" / name)
    (stage / "VERSION.txt").write_text(version + "\n", encoding="utf-8")
    (stage / "使用说明.txt").write_text(f"""rust-mhxy {version} - 梦幻西游抓鬼/鬼王任务坐标辅助
============================================================

一、运行
  1. 解压整个文件夹到可写目录，assets 目录和 DLL 要一起保留。
  2. 双击 rust-mhxy.exe 运行，启动时不会弹出命令窗口。

二、说明
  - 本地 PP-OCRv5 模型已随包放在 assets/models，离线可用。
  - 远端 PP-OCR：选择远端模式，填写服务地址与 Token，可点击「测试连接」。
  - 游戏窗口自动扫描并选中，也可通过下拉框手动选择。
  - 使用顶部按钮开始或暂停识别；任务状态显示在任务卡片标题右侧。
  - 「设置识别区域」打开独立窗口，可点击截图取点取色。
  - 日志始终开启，支持简单和详细两个级别。
  - 配置自动保存到程序目录 config.json，可参考 config.example.json。

三、运行库
  - 已自带 VC++ 运行库与 DirectML.dll，适用于 Windows 10/11 64 位。
""", encoding="utf-8-sig")

    archive = output / f"{stage.name}.zip"
    temporary_archive = archive.with_suffix(".zip.tmp")
    with zipfile.ZipFile(temporary_archive, "w", compression=zipfile.ZIP_DEFLATED) as bundle:
        for path in sorted(stage.rglob("*")):
            if path.is_file():
                bundle.write(path, path.relative_to(output).as_posix())
    with zipfile.ZipFile(temporary_archive) as bundle:
        bad_file = bundle.testzip()
        if bad_file is not None:
            raise ValueError(f"ZIP 文件损坏：{bad_file}")
    temporary_archive.replace(archive)
    checksum = archive.with_suffix(".zip.sha256")
    checksum.write_text(f"{sha256(archive)}  {archive.name}\n", encoding="utf-8")
    print(f"已生成：{archive}（{archive.stat().st_size / 1024 / 1024:.1f} MiB）", flush=True)
    print(f"SHA256：{checksum}", flush=True)
    return archive


def default_target_root() -> Path:
    """优先使用 CARGO_TARGET_DIR（CI 会把构建产物放到持久化缓存目录）。"""
    configured = os.environ.get("CARGO_TARGET_DIR")
    if not configured:
        return ROOT / "target"
    path = Path(configured)
    return path if path.is_absolute() else (ROOT / path)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--print-version", action="store_true", help="仅输出 Cargo 版本，含 v 前缀")
    parser.add_argument("--binary", type=Path,
                        default=default_target_root() / TARGET / "release" / "rust-mhxy.exe")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "dist")
    parser.add_argument("--models-dir", type=Path, default=Path(os.environ.get("OAR_HOME", Path.home() / ".oar")))
    parser.add_argument("--ort-cache", type=Path)
    args = parser.parse_args()
    with (ROOT / "Cargo.toml").open("rb") as manifest:
        version = "v" + tomllib.load(manifest)["package"]["version"]
    if args.print_version:
        print(version)
        return
    package(args.binary.resolve(), args.output_dir.resolve(), args.models_dir.resolve(),
            args.ort_cache or default_ort_cache(), version)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, zipfile.BadZipFile) as error:
        print(f"打包失败：{error}", file=sys.stderr)
        if os.environ.get("GITHUB_ACTIONS") == "true":
            message = str(error).replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
            print(f"::error title=Windows packaging failed::{message}")
        raise SystemExit(1)
