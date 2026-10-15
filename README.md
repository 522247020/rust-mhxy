# rust-mhxy

梦幻西游抓鬼 / 鬼王任务坐标辅助工具。自动识别任务栏中的地图与坐标，在地图上标记目标位置，让任务信息更直观。

基于 **Rust + egui** 开发，支持本地 PP-OCRv5 和远端 PP-OCR，提供 Windows 10 / 11 64 位桌面应用。

[![Windows build](https://github.com/522247020/rust-mhxy/actions/workflows/windows.yml/badge.svg)](https://github.com/522247020/rust-mhxy/actions/workflows/windows.yml)
[![Release](https://img.shields.io/github/v/release/522247020/rust-mhxy)](https://github.com/522247020/rust-mhxy/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## 功能特点

- **任务坐标识别**：识别抓鬼、捉鬼、鬼王任务文字，提取地图名称与坐标。
- **地图可视化**：内置 20 张地图，显示目标位置、附近范围和可选的 50 × 50 高概率区域；点击任务坐标即可复制。
- **本地与远端 OCR**：本地识别使用 PP-OCRv5，远端识别支持 `multipart` 和 `raw` 请求格式，并可直接测试连接。
- **自定义识别区域**：独立设置窗口支持主区域、备用区域和截图预览，适配不同游戏窗口布局。
- **图色门控**：按指定点位的颜色判断是否需要识别，未命中时暂停 OCR，减少无效识别。
- **双任务展示**：抓鬼与鬼王可上下或左右排列；状态提示与标题在同一行，状态变化时地图保持稳定。
- **桌面使用体验**：深色界面、可折叠设置栏、界面缩放、简单 / 详细日志，设置自动保存。

## 下载与使用

前往 [GitHub Releases](https://github.com/522247020/rust-mhxy/releases/latest) 下载 Windows 64 位压缩包。

当前版本为 **v0.1.0**，分发包名称为 `rust-mhxy-v0.1.0-windows-x86_64.zip`，同时提供 `.zip.sha256` 校验文件。

1. 将整个压缩包解压到可写目录，保留 `assets` 目录和所有 DLL。
2. 双击 `rust-mhxy.exe` 启动，程序不会弹出命令窗口。
3. 打开游戏，在设置栏中确认选中的游戏窗口；也可以通过下拉框手动选择或点击「刷新」。
4. 选择「本地 PP-OCR」或「远端 PP-OCR」，根据需要设置识别区域。
5. 点击顶部「开始识别」，在任务卡片中查看地图与坐标；点击「暂停识别」停止。

Windows 分发包包含本地 OCR 模型、地图、DirectML 和 VC++ 运行库，本地识别可离线使用。截图识别时请保持游戏窗口可见，避免最小化或遮挡任务栏区域。

### 识别方式

| 方式 | 使用说明 |
| --- | --- |
| 本地 PP-OCR | 使用 PP-OCRv5，首次识别时加载模型。优先读取 `assets/models/`，缺少模型时自动下载到 `~/.oar/`。 |
| 远端 PP-OCR | 填写服务地址与可选的 Bearer Token，点击「测试连接」确认服务可用。截图以 PNG 格式发送给服务。 |

远端服务支持两种请求格式：

- `multipart`：请求 `/ocr`，图片字段为 `file`。
- `raw`：请求 `/ocr/raw`，请求体为 PNG 图片字节。

服务地址填写根地址即可，例如 `http://192.168.1.100:8008`，程序会按请求格式补全接口路径。

### 设置识别区域

点击「设置识别区域」打开独立窗口：

- 点击「更新截图」获取当前游戏客户区预览。
- 按左、上、右、下调整主区域，单位为客户区百分比；红框表示主区域。
- 需要识别第二处任务文字时，启用备用区域；蓝框表示备用区域。
- 勾选「点击预览取点」，可直接从截图中添加颜色采样点。

默认主区域为客户区宽度的 `70%～99%`、高度的 `24%～70%`。若识别不到任务，可先检查截图范围，再切换详细日志查看识别过程。

### 图色门控

图色门控通过少量像素点判断任务栏是否处于可识别状态，适合在战斗或任务完成时跳过 OCR。

在区域设置窗口中取点后启用门控，可选择「全部命中」或「任一命中」，并调整每个点位的颜色容差与采样半径。默认每 `300ms` 检查一次；从未命中恢复为命中时，会立即触发一次识别。未配置点位时默认允许 OCR。

## 配置

大部分设置可直接在界面中调整，程序会自动保存到同目录的 `config.json`，下次启动时恢复。开发运行时，配置保存在项目根目录。

需要手动配置时，可将 [config.example.json](config.example.json) 复制为 `config.json`。配置支持 `//` 和 `/* */` 注释，修改后重启程序生效。

| 配置项 | 说明 |
| --- | --- |
| `ocr/mode` | `local` 本地识别，`remote` 远端识别 |
| `ocr/interval_seconds` | 识别间隔，范围为 1～60 秒 |
| `ocr/task_region` | 主区域，格式为 `左,上,右,下`，各值为 0～1 |
| `ocr/extra_task_region` | 备用区域，格式同主区域 |
| `ocr/extra_region_enabled` | 是否启用备用区域 |
| `api/url` | 远端 PP-OCR 服务地址 |
| `api/token` | Bearer Token，可留空 |
| `api/format` | `multipart` 或 `raw` |
| `task/option/鬼王` | 是否同时识别鬼王 |
| `task/high_chance` | 是否显示高概率区域 |
| `task/horizontal_layout` | `false` 上下排列，`true` 左右排列 |
| `color_gate/enabled` | 是否启用图色门控 |
| `color_gate/interval_ms` | 门控采样间隔，默认 300 毫秒 |
| `color_gate/mode` | `all` 全部命中，`any` 任一命中 |
| `color_gate/tolerance` | 新增点位的默认颜色容差 |
| `color_gate/points` | 点位数组，支持坐标、颜色、容差、半径和标签 |
| `log/level` | `simple` 简单日志，`detailed` 详细日志 |
| `window/sidebar_expanded` | 是否展开设置栏 |
| `window/scale` | `0` 跟随系统 DPI，`0.75`～`2.5` 固定缩放，默认 `1.0` |
| `window/coordinate/on_top` | 是否置顶窗口 |

## 开发与构建

### 从源码运行

安装当前稳定版 [Rust](https://rustup.rs/)，克隆仓库后运行：

```bash
git clone https://github.com/522247020/rust-mhxy.git
cd rust-mhxy
cargo run --locked
```

Windows 使用 Win32 API 截取游戏客户区，编译需要 Visual Studio C++ 构建工具。Linux / macOS 使用 `xcap` 提供截图，主要用于开发调试，需具备相应的图形环境与系统依赖。

首次构建时会下载 ONNX Runtime。直接从源码运行时，如果没有本地 OCR 模型，首次本地识别会自动下载；远端模式不会加载本地模型。

开发检查：

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

需要下载模型或提供真实任务截图的两项 OCR 集成测试默认忽略。

### 构建 Windows 分发包

打包脚本需要 **Python 3.11+**，会自动准备并校验离线模型和运行库，检查 EXE 的 x64 架构及 GUI 子系统，生成 ZIP 与 SHA256 校验文件。

在 Windows 上构建：

```powershell
cargo build --locked --release --target x86_64-pc-windows-msvc
python scripts/package-windows.py
```

在 Linux 上交叉编译：

```bash
cargo install cargo-xwin --locked
rustup target add x86_64-pc-windows-msvc
bash scripts/package-windows.sh
```

产物保存在 `dist/`，文件名中的版本号自动读取 `Cargo.toml`。

### 自动构建与发布

[Windows build](https://github.com/522247020/rust-mhxy/actions/workflows/windows.yml) 工作流在 GitHub 的 Windows 环境中构建：

- 推送到 `main`、向 `main` 提交 Pull Request，或手动运行工作流：执行格式检查、编译、打包、Clippy 和测试，成功后上传构建产物，保留 30 天。
- 推送 `v*` 标签：校验标签与 Cargo 版本一致，所有检查通过后自动创建 GitHub Release，上传 Windows ZIP 和 SHA256 文件。

发布新版本时，先更新 `Cargo.toml` 的 `package.version` 和 `Cargo.lock`，提交后推送相应标签。工作流使用 GitHub 自带的 `GITHUB_TOKEN` 发布，无需额外配置凭据。

## 项目结构

```text
src/
  main.rs       # 应用启动、版本信息与中文字体
  app.rs        # 界面、任务状态与识别调度
  theme.rs      # 配色与通用界面组件
  core.rs       # 任务解析与地图坐标计算
  capture.rs    # 窗口枚举与截图
  gate.rs       # 图色门控
  ocr.rs        # 本地与远端 OCR
  config.rs     # 配置读写
assets/
  maps.json     # 地图定义
  maps/         # 地图图片
scripts/
  package-windows.py  # Windows 分发包组装
  package-windows.sh  # Linux 交叉编译入口
.github/workflows/
  windows.yml   # Windows 自动构建与发布
config.example.json
LICENSE
```

## 开源协议

本项目采用 [MIT License](LICENSE) 开源协议。
