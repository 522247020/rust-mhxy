#!/usr/bin/env python3
"""生成静态下载站（index.html + latest.json + 安装包）。

用法示例：
    python scripts/build-site.py --notes "修复坐标抖动；新增自动更新"
    python scripts/build-site.py --notes-file site/notes.txt

生成结果在 dist/site/，把里面全部文件上传到站点根目录即可。默认站点地址：
https://xyq.mhwy.work ，安装包会被放到 /downloads/ 下。
"""

import argparse
import hashlib
import html
import json
import shutil
import tomllib
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_BASE_URL = "https://xyq.mhwy.work"
DEFAULT_REPO = "https://github.com/522247020/rust-mhxy"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def notes_html(notes: str) -> str:
    lines = [line.strip().lstrip("-*").strip() for line in notes.splitlines()]
    lines = [line for line in lines if line]
    if not lines:
        lines = ["本次更新以修复和优化为主。"]
    return "\n".join(f"      <li>{html.escape(line)}</li>" for line in lines)


def render(template: str, values: dict[str, str]) -> str:
    output = template
    for key, value in values.items():
        output = output.replace("{{" + key + "}}", value)
    remaining = [key for key in values if "{{" + key + "}}" in output]
    if remaining:
        raise SystemExit(f"模板里仍有未替换的占位符：{remaining}")
    return output


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--zip", type=Path, help="安装包路径，默认使用 dist 下当前版本的 zip")
    parser.add_argument("--notes", default="", help="更新说明，换行分隔")
    parser.add_argument("--notes-file", type=Path, help="从文件读取更新说明")
    parser.add_argument("--base-url", default=DEFAULT_BASE_URL, help="站点根地址")
    parser.add_argument("--repo", default=DEFAULT_REPO, help="GitHub 仓库地址")
    parser.add_argument("--site-dir", type=Path, default=ROOT / "site")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "dist" / "site")
    args = parser.parse_args()

    version = "v" + tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    zip_path = args.zip or ROOT / "dist" / f"rust-mhxy-{version}-windows-x86_64.zip"
    if not zip_path.is_file():
        raise SystemExit(f"找不到安装包：{zip_path}（先运行 scripts/package-windows.sh）")

    notes = args.notes
    if args.notes_file:
        notes = args.notes_file.read_text(encoding="utf-8")
    notes = notes.strip() or "常规更新与修复。"

    base_url = args.base_url.rstrip("/")
    zip_name = zip_path.name
    download_url = f"{base_url}/downloads/{zip_name}"
    digest = sha256(zip_path)
    size = zip_path.stat().st_size
    date = datetime.now(timezone.utc).strftime("%Y-%m-%d")

    output = args.output_dir
    downloads = output / "downloads"
    downloads.mkdir(parents=True, exist_ok=True)
    shutil.copy2(zip_path, downloads / zip_name)
    for asset in ("style.css", "favicon.svg"):
        shutil.copy2(args.site_dir / asset, output / asset)

    template = (args.site_dir / "index.html").read_text(encoding="utf-8")
    page = render(
        template,
        {
            "VERSION": version,
            "DATE": date,
            "SIZE_MB": f"{size / 1024 / 1024:.1f}",
            "SHA256": digest,
            "ZIP_NAME": html.escape(zip_name),
            "DOWNLOAD_URL": html.escape(download_url, quote=True),
            "NOTES_HTML": notes_html(notes),
            "REPO": html.escape(args.repo, quote=True),
            "YEAR": str(datetime.now().year),
        },
    )
    (output / "index.html").write_text(page, encoding="utf-8")

    manifest = {
        "version": version,
        "notes": notes,
        "published_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": {
            "windows-x86_64": {
                "url": download_url,
                "sha256": digest,
                "size": size,
            }
        },
    }
    (output / "latest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )

    print(f"版本：{version}")
    print(f"安装包：{zip_name}（{size / 1024 / 1024:.1f} MB）")
    print(f"SHA256：{digest}")
    print(f"清单：{output / 'latest.json'}")
    print(f"页面：{output / 'index.html'}")
    print()
    print(f"把 {output}/ 里的全部文件上传到站点根目录（{base_url}）：")
    print("  index.html  style.css  favicon.svg  latest.json  downloads/")
    print("latest.json 建议设置 Cache-Control: no-cache，安装包可以长缓存。")


if __name__ == "__main__":
    main()
