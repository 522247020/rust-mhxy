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
- **桌面使用体验**：暗色 / 亮色双主题、顶部按钮统一样式、设置栏各分组可折叠、窗口置顶开关、简单 / 详细日志，设置自动保存。
- **自适应布局**：收起设置栏后窗口可以继续缩小；日志高度随窗口高度自动扩展。
- **自动更新**：默认开启，启动后静默检查 `xyq.mhwy.work` 上的新版本；右上角按钮会从「检查更新」变成「更新到 vX」，点击即下载、校验 SHA256 并自动覆盖重启，全程不弹窗打扰。

## 下载与使用

前往 [xyq.mhwy.work](https://xyq.mhwy.work) 或 [GitHub Releases](https://github.com/522247020/rust-mhxy/releases/latest) 下载 Windows 64 位压缩包。国内访问自建站点通常更快。

当前版本为 **v0.2.0**，分发包名称为 `rust-mhxy-v0.2.0-windows-x86_64.zip`，同时提供 `.zip.sha256` 校验文件。

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

点击「设置识别区域」打开独立窗口（优先定位在主窗口右侧，右侧空间不足时改放左侧；两边都放不下才贴屏幕右缘，并跟随「置顶」开关保持在其他窗口之上）。窗口使用**左右布局**：

- 顶部是**识别方案**栏：下拉切换、保存、另存为、删除；方案同时包含图色门控与预处理设置，切换后立即生效。
- **上排**：左边「截图范围」和「图色门控设置」，右边「识别区域预览」（红框=主区域，蓝框=备用区域）。
- 「截图范围」内：主区域标题单独一行、左/上/右/下分两行两列；下面是备用区域标题，单独一行「启用备用区域」，启用后同样两行两列。
- **下排**：「识别预处理」｜「预处理预览」两栏分割；预处理里主区域设置在上、备用区域设置在下，未启用备用区域时备用区域整块灰色不可选。预处理预览里**主区域和备用区域左右排列**，可以直接对比处理效果。
- 点击预览图可取点，也可以取文字颜色；测试 OCR 的结果显示在窗口顶部。
- 点击「更新截图」获取当前游戏客户区预览；点击预览图可取点（或取文字颜色）。
- 按左、上、右、下调整主区域，单位为客户区百分比。
- 需要识别第二处任务文字时，启用备用区域。**通用任务**的坐标不一定出现在主区域里，勾选该任务后会自动启用备用区域，两块截图会上下拼接后一起送 OCR。

默认主区域为客户区宽度的 `70%～99%`、高度的 `24%～70%`。若识别不到任务，可先检查截图范围，再切换详细日志查看识别过程。

### 图色门控

图色门控通过少量像素点判断任务栏是否处于可识别状态，适合在战斗或任务完成时跳过 OCR。

在区域设置窗口中取点后启用门控，可选择「全部命中」或「任一命中」，并调整每个点位的颜色容差与采样半径。默认每 `300ms` 检查一次；从未命中恢复为命中时，会立即触发一次识别。未配置点位时默认允许 OCR。

### 识别预处理

任务栏背景（花纹、动画、彩色底图）会干扰 OCR，容易把坐标认错或让点位来回跳。识别预处理会在送入 OCR 前先处理任务区域截图，四个步骤可以任意叠置，按固定顺序执行：

| 步骤 | 说明 |
| --- | --- |
| 颜色提取 | 任务文字颜色稳定时最干净：只保留指定颜色附近的像素，直接输出**黑字白底**（自带反色，白色文字也不会消失） |
| 灰度 | 去掉颜色干扰，保留笔画细节 |
| 二值化 | 推荐。Otsu 自动阈值转为白底黑字，按边框判断背景亮暗，深色或浅色任务栏都适用（内部已先转灰度） |
| 反色 | 最后一步，把黑白颠倒；二值化把文字判成白色时才需要 |

常见组合：`二值化`（日常首选）、`颜色提取 + 二值化`（背景花、文字颜色稳定时最干净）、`灰度 + 二值化`（与只选二值化等同）。如果预览里文字是白色、背景是黑色，勾上「反色」即可。

在「设置识别区域」窗口的「识别预处理」里，先选择「主区域」或「备用区域」，再分别勾选步骤——两块区域的预处理设置**互相独立**（管线、文字颜色、容差各自保存）。识别时主区域和备用区域会先按各自设置处理，再拼接送 OCR；右侧两张预览分别显示处理后的效果。工具条上的 **测试 OCR** 按钮会用当前截图和预处理设置识别一次，并直接显示耗时、识别文字与解析出的任务，方便判断当前设置是否合适，不影响正式识别。

选「颜色提取」时，文字颜色有两种指定方式：

- **色值输入**：在「色值」里填 `#RRGGBB`，默认 `#eaff65`（即 RGB `234, 255, 101`），点「默认」可恢复；「更多颜色」里还有内嵌调色板。
- **取色**：「点击预览取色」后在右侧预览图里点任务文字；也可以用「用图色点位颜色」复用图色门控的取点。取色对像素位置比较敏感，点不准时以「色值」为准。

设置区右侧会实时显示处理后的任务区域，便于对照调整。颜色容差控制允许多大的颜色偏差（背景与文字颜色接近时适当调小）。

预处理后的截图也会参与“识别区域是否变化”的比较，因此背景里的动画不容易反复触发 OCR，坐标也会更稳定。如果仍出现个位数跳动，可以配合「大字体模式」使用。

## 更新日志

### v0.2.0

- 新增 **OCR 预处理**：颜色提取、灰度、二值化、反色四个步骤可任意叠置（固定顺序：颜色提取 → 灰度 → 二值化 → 反色），降低游戏背景对识别的干扰；预处理后的截图参与“是否变化”的比较，背景动画不再反复触发 OCR。
- 颜色提取直接输出黑字白底（自带反色），避免白色任务文字被涂白后消失；二值化改为按截图边框判断背景亮暗，文字占比很大时也不易黑白颠倒（必要时可用「反色」）。
- 取点窗口新增「识别预处理」设置区：左侧参数、右侧实时预处理预览；文字颜色默认 `#eaff65`（RGB `234, 255, 101`），支持色值输入、内嵌调色板、预览点击取色和图色点位取色。
- 取点窗口新增 **测试 OCR** 按钮：用当前截图和预处理设置识别一次，显示耗时、识别文字和解析结果，不影响正式识别。
- 侧栏「识别方式」显示当前生效的预处理步骤。
- 新增**暗色 / 亮色双主题**：侧栏「界面」里切换，配置保存为 `window/theme`，启动时恢复。
- 侧栏分组（游戏窗口、识别模式、识别方式、任务显示、界面、日志）都可以折叠，折叠状态保存到配置；顶部按钮统一为同一套样式与高度。
- 识别区域窗口改为三段布局：上排「截图范围/门控设置 | 识别区域预览」，下排「预处理 | 备用区域 | 预处理预览」。
- 新增**识别方案**：图色门控 + 预处理设置可保存为多个方案，一键切换。
- 主区域与备用区域的预处理**分开设置、分开处理**：各自有独立的管线、文字颜色和容差，识别前分别处理再拼接。
- 「识别区域」窗口的三个设置分组支持折叠，折叠状态保存到配置。
- 新增**自动更新**：从 `xyq.mhwy.work` 读取清单、下载安装包并校验 SHA256，自动解压覆盖后重启；右上角新增「检查更新」按钮（发现新版本时变色提示），底部右下角新增发布页入口。
- 新增静态下载页与 `scripts/build-site.py`，一条命令生成 `index.html`、`latest.json` 和安装包。
- 新增 Gitea 工作流与 `scripts/deploy-site.sh`，推送标签即可构建并同步下载站。

### v0.1.6

- 修复坐标超出地图范围时，黄色「附近」范围框跟着贴边点偏移的问题；现在按**实际任务坐标**绘制，超出地图的部分由地图边界裁剪。
- 贴边显示、半径 50 等行为与 v0.1.5 一致。

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
| `ocr/preprocess` | OCR 前预处理，可叠加（逗号分隔）：`color` 颜色提取、`gray` 灰度、`binary` 二值化、`invert` 反色，如 `binary`、`color,binary` |
| `ocr/preprocess_color` | 按颜色提取时保留的文字颜色（十六进制） |
| `ocr/preprocess_tolerance` | 按颜色提取时的颜色容差，0～255 |
| `ocr/extra_preprocess` | 备用区域独立的预处理管线，取值同 `ocr/preprocess` |
| `ocr/extra_preprocess_color` | 备用区域颜色提取的文字颜色 |
| `ocr/extra_preprocess_tolerance` | 备用区域颜色提取的容差 |
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
| `profiles/active` | 当前识别方案名称 |
| `profiles/items` | 保存的识别方案数组（图色门控 + 预处理设置） |
| `log/level` | `simple` 简单日志，`detailed` 详细日志 |
| `update/enabled` | 启动后是否自动检查更新 |
| `update/manifest_url` | 更新清单地址，默认 `https://xyq.mhwy.work/latest.json` |
| `window/sidebar_expanded` | 是否展开设置栏 |
| `window/theme` | 界面主题：`dark` 暗色 / `light` 亮色 |
| `window/sidebar/folded` | 侧栏折叠的分组，逗号分隔：`window` / `trigger` / `ocr` / `task` / `ui` / `log` |
| `window/region/scope_open` | 识别区域窗口「截图范围」分组是否展开 |
| `window/region/gate_open` | 识别区域窗口「图色门控」分组是否展开 |
| `window/region/preprocess_open` | 识别区域窗口「识别预处理」分组是否展开 |
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

### 自动更新与下载站

应用默认开启自动更新：启动约 2 秒后静默检查一次，之后**每 30 分钟**检查一次（可在配置里关闭或修改地址）。状态都在右上角同一个按钮上，不会主动弹窗打扰：

| 按钮状态 | 点击后 |
| --- | --- |
| 检查更新 | 立即检查 |
| 检查中… | — |
| 已是最新版本 / 检查失败（短暂提示） | 立即检查 |
| 发现新版本 vX（金色） | 弹出更新说明，可查看更新内容并下载 |
| 下载 45% | 打开下载进度，可取消 |
| 重启并更新（金色） | 打开更新说明，确认后覆盖程序目录并重启 |

发现新版本时，按钮会变成金色；点击后弹出的窗口里显示更新内容和安装包大小。检查失败时，底栏会直接显示具体原因（地址、清单格式、缺少下载地址等），不再静默失败。点「下载并更新」开始下载（完成后校验 SHA256），再点「立即重启并更新」即可自动覆盖并重启。自动替换仅在 Windows 生效，程序目录需要可写；失败时按钮会短暂提示「检查失败」，悬停可看原因，也可以从底栏「发布页」手动下载。

底栏的「发布页」「GitHub」以及说明窗口里的链接，都会用系统默认浏览器打开。

发布新版本并同步下载站：

```bash
bash scripts/package-windows.sh
python scripts/build-site.py --notes "本次更新内容，一行一条"
```

`dist/site/` 里是完整的静态站点，整体上传到 `xyq.mhwy.work` 的站点根目录即可：

```text
index.html  style.css  favicon.svg  latest.json  downloads/
```

`latest.json` 是应用读取的更新清单（页面源码在 `site/`，由脚本填充版本与校验值）：

```json
{
  "version": "v0.1.7",
  "notes": "修复坐标抖动；新增自动更新",
  "platforms": {
    "windows-x86_64": {
      "url": "https://xyq.mhwy.work/downloads/rust-mhxy-v0.1.7-windows-x86_64.zip",
      "sha256": "…",
      "size": 44369540
    }
  }
}
```

建议给 `latest.json` 设置 `Cache-Control: no-cache`（应用请求时也会自动加时间戳参数），安装包可以长期缓存。
建议给 `latest.json` 设置 `Cache-Control: no-cache`（应用请求时也会自动加时间戳参数），安装包可以长期缓存。

#### 发布时自动部署站点

推送 `v*` 标签后，[Windows build](https://github.com/522247020/rust-mhxy/actions/workflows/windows.yml) 除了创建 GitHub Release，还会自动：

1. 用刚构建好的安装包生成完整站点（`index.html`、`latest.json`、`downloads/`）；
2. 把站点作为 Actions 产物上传（名称 `rust-mhxy-site-vX.Y.Z`，保留 14 天，可手动下载）；
3. 配置了服务器密钥时，通过 `rsync` 覆盖到站点目录；
4. 部署后请求一次 `latest.json`，确认线上版本已更新（CDN 缓存导致未生效时只给警告，不中断发布）。

在仓库 **Settings → Secrets and variables → Actions** 添加：

| Secret | 说明 | 示例 |
| --- | --- | --- |
| `SITE_SSH_HOST` | 服务器地址 | `xyq.mhwy.work` |
| `SITE_SSH_USER` | SSH 用户 | `deploy` |
| `SITE_SSH_PORT` | SSH 端口，可留空（默认 22） | `22` |
| `SITE_SSH_KEY` | 可免密登录的 OpenSSH 私钥 | `-----BEGIN OPENSSH PRIVATE KEY-----…` |
| `SITE_PATH` | 站点目录，部署会 `rsync --delete` 覆盖它 | `/var/www/xyq.mhwy.work` |

服务器要求：能被 GitHub Actions 通过 SSH 访问、安装 `rsync`、目录可写。`SITE_PATH` 请使用**独立目录**，不要直接指向包含其他站点的 web 根目录。

只配置 `SITE_SSH_HOST` 以外的密钥不完整时会报错；完全不配置时自动跳过部署，只保留 GitHub Release 和站点产物。

**更新说明的来源**：默认使用标签注释，例如 `git tag -a v0.1.7 -m "修复坐标抖动；新增自动更新"`；想单独维护时，在仓库放 `site/notes.txt`（一行一条，参考 `site/notes.example.txt`），工作流会优先使用它。

**nginx 缓存建议**：

```nginx
location = /latest.json {
    add_header Cache-Control "no-cache";
}
location /downloads/ {
    expires 30d;
}
```

如果服务器无法被 GitHub 主动 SSH（例如只允许国内 IP 访问），可以改成「拉取式」：让 Actions 把 `dist/site` 推送到仓库的 `site` 分支，服务器用定时任务 `git pull` 后同步到 web 目录。

#### 从 Gitea 构建并部署

国内网络下也可以让 Gitea 承担构建和部署，工作流在 [`.gitea/workflows/site.yml`](.gitea/workflows/site.yml)：

- 推送 `v*` 标签或手动运行「Build and deploy download site」触发；
- 用 `scripts/package-windows.sh` 交叉编译 Windows 包，再用 `scripts/build-site.py` 生成站点；
- 用 `scripts/deploy-site.sh` 通过 rsync 同步到网站目录；
- 没配置 `SITE_SSH_HOST` 时只构建并保留 `dist/site`，不会失败。

Gitea Runner 需要（自建、Linux）：Rust + `x86_64-pc-windows-msvc` target、`cargo-xwin`、Python 3.11+、`rsync`，并且能访问 crates.io / 模型下载源。Secrets 与 GitHub 相同，在 Gitea 仓库 **设置 → Actions → Secrets** 里添加 `SITE_SSH_HOST`、`SITE_SSH_USER`、`SITE_SSH_PORT`、`SITE_SSH_KEY`、`SITE_PATH`。

也可以在任何机器上手动部署已生成的站点：

```bash
bash scripts/package-windows.sh
python scripts/build-site.py --notes "本次更新内容"
SITE_SSH_HOST=xyq.mhwy.work SITE_SSH_USER=deploy SITE_PATH=/var/www/xyq.mhwy.work   SITE_SSH_KEY=~/.ssh/deploy_key bash scripts/deploy-site.sh dist/site
```

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
  update.rs     # 自动更新：清单检查、下载校验与替换
assets/
  maps.json     # 地图定义
  maps/         # 地图图片
scripts/
  package-windows.py  # Windows 分发包组装
  build-site.py       # 生成静态下载页与更新清单
  package-windows.sh  # Linux 交叉编译入口
.github/workflows/
  windows.yml   # Windows 自动构建与发布
config.example.json
site/
  index.html    # 下载页模板（占位符由 build-site.py 填充）
  style.css     # 页面样式
  favicon.svg   # 站点图标
LICENSE
```

## 开源协议

本项目采用 [MIT License](LICENSE) 开源协议。
