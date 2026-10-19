# rust-mhxy

梦幻西游抓鬼 / 鬼王任务坐标辅助工具。自动识别任务栏中的地图与坐标，在地图上标记目标位置，让任务信息更直观。

基于 **Rust + egui** 开发，支持本地 PP-OCRv5 和远端 PP-OCR，提供 Windows 10 / 11 64 位桌面应用。

[![Windows build](https://github.com/522247020/rust-mhxy/actions/workflows/windows.yml/badge.svg)](https://github.com/522247020/rust-mhxy/actions/workflows/windows.yml)
[![Release](https://img.shields.io/github/v/release/522247020/rust-mhxy)](https://github.com/522247020/rust-mhxy/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## 功能特点

- **任务坐标识别**：识别抓鬼、捉鬼、鬼王任务文字，提取地图名称与坐标；另有**通用任务**模式，不看任务类型，只按「地图名 + 坐标」标注。
- **坐标容错**：OCR 把坐标数字换行拆开（如 `79,26` 变成 `79,2` + 换行 + `6`）时会自动拼接；并对「去 … 附近」和「去 … 处」两种任务描述做二次结构校验，避免少识别一位数字。
- **地图可视化**：内置 20 张地图，显示目标位置与附近范围；点击任务面板只展示地图，未识别到任务时保留上次结果。
- **捉鬼高概率区域**：抓鬼和鬼王各自可勾选，按 175LG 省眼规则在地图上叠加 50×50 的高概率格子（普陀山、五庄观会额外多画一个）；格子只显示落在黄色「附近」范围框内的部分，不会超出黄框。
- **本地与远端 OCR**：本地识别使用 PP-OCRv5，远端识别支持 `multipart` 和 `raw` 请求格式，并可直接测试连接。
- **手动识别模式**：可在「自动（按间隔）」和「手动（快捷键）」之间切换；手动模式下按快捷键触发一次识别，快捷键支持界面录制，Windows 下全局生效（游戏窗口在前台也能触发）。
- **自定义识别区域**：独立设置窗口支持主区域、备用区域和截图预览，适配不同游戏窗口布局。
- **图色门控**：按指定点位的颜色判断是否需要识别，未命中时暂停 OCR，减少无效识别。
- **多任务展示**：抓鬼、鬼王、通用任务可上下或左右排列；任务区域只显示地图，不占用面积显示任务名和识别状态。
- **桌面使用体验**：深色界面、可折叠设置栏、窗口置顶开关、简单 / 详细日志，设置自动保存。
- **自适应布局**：收起设置栏后窗口可以继续缩小；日志高度随窗口高度自动扩展。

## 下载与使用

前往 [GitHub Releases](https://github.com/522247020/rust-mhxy/releases/latest) 下载 Windows 64 位压缩包。

当前版本为 **v0.1.5**，分发包名称为 `rust-mhxy-v0.1.5-windows-x86_64.zip`，同时提供 `.zip.sha256` 校验文件。

1. 将整个压缩包解压到可写目录，保留 `assets` 目录和所有 DLL。
2. 双击 `rust-mhxy.exe` 启动，程序不会弹出命令窗口。
3. 打开游戏，在设置栏中确认选中的游戏窗口；也可以通过下拉框手动选择或点击「刷新」。
4. 选择「本地 PP-OCR」或「远端 PP-OCR」，根据需要设置识别区域。
5. 点击顶部「开始识别」，在地图中查看地图与坐标；点击「暂停识别」停止。
6. 需要让本窗口始终显示在游戏前面时，点击顶部的「置顶」按钮，或在设置栏「界面」中勾选「窗口始终置顶」。

Windows 分发包包含本地 OCR 模型、地图、DirectML 和 VC++ 运行库，本地识别可离线使用。截图识别时请保持游戏窗口可见，避免最小化或遮挡任务栏区域。

### 识别模式

在设置栏「识别模式」中选择：

- **自动（按间隔）**：按「识别间隔」定时截图识别，配合图色门控可跳过战斗 / 任务完成时的无效识别。
- **手动（快捷键）**：不会定时识别；点顶部「开始监听」后，每按一次快捷键就识别一次。

手动模式支持在输入框直接填写快捷键（如 `F8`、`Ctrl+F8`），也可以点「录制」后直接按一次想要的组合键。Windows 下通过系统全局快捷键注册，游戏窗口在前台时也能触发；注册失败或非 Windows 平台会退回到「窗口聚焦时可用」，并写入详细日志。手动触发会忽略图色门控。

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

点击「设置识别区域」打开独立窗口（优先定位在主窗口右侧，右侧空间不足时改放左侧；两边都放不下才贴屏幕右缘，并跟随「置顶」开关保持在其他窗口之上）：

- 点击「更新截图」获取当前游戏客户区预览。
- 按左、上、右、下调整主区域，单位为客户区百分比；红框表示主区域。
- 需要识别第二处任务文字时，启用备用区域；蓝框表示备用区域。**通用任务**的坐标不一定出现在主区域里，勾选该任务后会自动启用备用区域，两块截图会上下拼接后一起送 OCR。
- 勾选「点击预览取点」，可直接从截图中添加颜色采样点。

默认主区域为客户区宽度的 `70%～99%`、高度的 `24%～70%`。若识别不到任务，可先检查截图范围，再切换详细日志查看识别过程。

### 图色门控

图色门控通过少量像素点判断任务栏是否处于可识别状态，适合在战斗或任务完成时跳过 OCR。

在区域设置窗口中取点后启用门控，可选择「全部命中」或「任一命中」，并调整每个点位的颜色容差与采样半径。默认每 `300ms` 检查一次；从未命中恢复为命中时，会立即触发一次识别。未配置点位时默认允许 OCR。

## 更新日志

### v0.1.5

- 「附近」范围框半径从 25 改为 **50**（边长 100），50×50 的高概率格子可以完整落在黄框内。
- 坐标超出地图范围时不再直接丢弃：**50% 以内贴边显示**，并在详细日志提示；超过 50% 才当作 OCR 噪声忽略。
- 修复部分任务（如普陀山 57,94）因地图标定范围偏小而被判「坐标无效」的问题。
- 高概率区域维持原版 175LG 规则（小坐标在所有地图上都会多画一个格子）。

### v0.1.4

- 新增**大字体模式**开关：OCR 把多位坐标截断导致 y 轴抖动时开启，同一 x 下大数字权重 3、小数字权重 1，持平显示大数字；x 或地图变化会重新绘制。
- 顶部新增**说明**按钮：包含基本流程、识别模式、图色门控、日志和项目地址等说明。
- 底部状态栏新增可点击的 **GitHub** 链接；手动模式下底栏显示快捷键，不再显示「每 X 秒识别」。
- GitHub Actions 改为**只在推送 `v*` 标签时**构建和发布，普通 `main` 推送和 PR 不再触发。
- README 与配置示例同步。

### v0.1.3

- 新增**手动识别模式**：可在「自动（按间隔）」和「手动（快捷键）」之间切换；手动模式下按快捷键触发一次 OCR。
- 快捷键支持界面填写或「录制」，Windows 下通过 `RegisterHotKey` 全局生效（游戏窗口在前台也能触发）；注册失败或非 Windows 平台回退为窗口聚焦时可用。
- 恢复**捉鬼高概率区域**：抓鬼和鬼王各自独立开关，按 175LG 省眼规则叠加 50×50 格子。
- 高概率格子只显示落在黄色「附近」范围框内的部分，不再超出黄框。
- 侧栏「任务显示」增加高概率区域开关；抓鬼 / 鬼王任务卡片顶部也可直接切换。

### v0.1.2

- 修复坐标少识别一位数字：OCR 把 `79,26` 拆成 `79,2` + 换行 + `6` 时，现在会正确拼接为 `(79, 26)`。
- 新增「去 … 附近 / 去 … 处」结构二次校验：取两者之间的连续文字再解析地图与坐标，兼容 OCR 的换行和空格噪声。
- 补充坐标解析回归测试。

### v0.1.1

- 新增通用任务模式，任务面板改为可选开关。
- 任务区只显示地图；取点窗口自动定位；字体改为内存映射。
- 重写项目介绍并补充 MIT 许可证。

### v0.1.0

- 首个版本：抓鬼 / 鬼王坐标识别、本地 / 远端 PP-OCR、地图可视化、图色门控等。

## 配置

大部分设置可直接在界面中调整，程序会自动保存到同目录的 `config.json`，下次启动时恢复。开发运行时，配置保存在项目根目录。

需要手动配置时，可将 [config.example.json](config.example.json) 复制为 `config.json`。配置支持 `//` 和 `/* */` 注释，修改后重启程序生效。

| 配置项 | 说明 |
| --- | --- |
| `ocr/mode` | `local` 本地识别，`remote` 远端识别 |
| `ocr/interval_seconds` | 识别间隔，范围为 1～60 秒（仅自动模式） |
| `ocr/trigger` | `auto` 按间隔自动识别，`manual` 快捷键手动触发 |
| `ocr/hotkey` | 手动模式的快捷键，如 `F8`、`Ctrl+F8`，Windows 下全局生效 |
| `ocr/task_region` | 主区域，格式为 `左,上,右,下`，各值为 0～1 |
| `ocr/extra_task_region` | 备用区域，格式同主区域 |
| `ocr/extra_region_enabled` | 是否启用备用区域 |
| `api/url` | 远端 PP-OCR 服务地址 |
| `api/token` | Bearer Token，可留空 |
| `api/format` | `multipart` 或 `raw` |
| `task/option/抓鬼` | 是否显示抓鬼面板 |
| `task/option/鬼王` | 是否同时识别鬼王 |
| `task/option/通用` | 通用任务：不看任务类型，只按地图名 + 坐标标注（与抓鬼/鬼王互斥） |
| `task/horizontal_layout` | `false` 上下排列，`true` 左右排列 |
| `task/high_chance` | 抓鬼地图是否叠加 50×50 高概率区域 |
| `task/high_chance/鬼王` | 鬼王地图是否叠加 50×50 高概率区域 |
| `task/large_font_mode` | 大字体模式：抑制 OCR 截断造成的 y 轴抖动（默认关闭） |
| `color_gate/enabled` | 是否启用图色门控 |
| `color_gate/interval_ms` | 门控采样间隔，默认 300 毫秒 |
| `color_gate/mode` | `all` 全部命中，`any` 任一命中 |
| `color_gate/tolerance` | 新增点位的默认颜色容差 |
| `color_gate/points` | 点位数组，支持坐标、颜色、容差、半径和标签 |
| `log/level` | `simple` 简单日志，`detailed` 详细日志 |
| `window/sidebar_expanded` | 是否展开设置栏 |
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

[Windows build](https://github.com/522247020/rust-mhxy/actions/workflows/windows.yml) 工作流**只在推送 `v*` 标签时触发**（普通 `main` 推送和 Pull Request 不会构建）：

- 校验标签与 `Cargo.toml` 版本一致；
- 执行格式检查、Windows 编译、打包、Clippy 和测试；
- 全部通过后自动创建 GitHub Release，上传 Windows ZIP 和 SHA256 文件。

发布新版本时，先更新 `Cargo.toml` 的 `package.version` 和 `Cargo.lock`，提交并推送 `main`，再推送同名的 `vX.Y.Z` 标签。工作流使用 GitHub 自带的 `GITHUB_TOKEN` 发布，无需额外配置凭据。

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
