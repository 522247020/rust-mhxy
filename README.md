# rust-mhxy

梦幻西游任务坐标辅助的 **Rust + egui** 重写版。

只保留最基础的两件事：

- **抓鬼 / 鬼王** 任务坐标识别与地图可视化
- **本地 PP-OCR** 与 **远端 PP-OCR** 两种识图方式

原 Python 版里的降妖伏魔、打宝图、计算器、答题器、计时器、闹钟、资料查询、消耗计算、
窗口置顶设置等全部砍掉。

## 功能

- 枚举游戏窗口（优先「梦幻西游 / MHXY」，否则取任意可用窗口），自动选中，也可用下拉框手动选、点「刷新」重扫
- 截取任务栏区域（默认客户区 `70%~99% 宽 / 24%~70% 高`），可选备用区域
- 本地识图：内置 **PP-OCRv5**（oar-ocr + ONNX Runtime），首次识别才加载模型，远端模式不加载；优先用随包 `assets/models` 里的模型，找不到时自动下载
- 远端识图：把截图 PNG 直接发给 PP-OCR 服务，支持 `multipart`（`/ocr`）和 `raw`（`/ocr/raw`），界面里有「测试连接」按钮
- 解析「抓鬼 / 捉鬼 / 鬼王」文字，匹配地图与坐标
- **图色门控**：每 300ms 比对配置的点位颜色，未命中（战斗 / 任务完成）就跳过 OCR，省 CPU
- 地图上标出坐标点、附近范围（`附近` 关键字）、捉鬼高概率区域（50×50）
- 配置写入 `config.json`（支持注释，兼容 Python 版字段）

## 构建与运行

Windows 是主要目标平台（窗口客户区截图用 Win32 API 精确实现）。

```bash
cd rust-mhxy
cargo run
```

首次构建时：

- `ort` 会下载 ONNX Runtime 二进制
- 首次执行本地识图时，模型（约 25MB）会下载到 `~/.oar/`

Linux/macOS 下会用 `xcap` 做开发期的近似截图，便于调试界面。

## Windows 打包（在 Linux 上交叉编译）

用 [`cargo-xwin`](https://github.com/rust-cross/cargo-xwin) 交叉编译到 `x86_64-pc-windows-msvc`：

```bash
cargo install cargo-xwin --locked
rustup target add x86_64-pc-windows-msvc
cargo xwin build --release --target x86_64-pc-windows-msvc
```

可直接使用项目脚本构建并组装完整分发包（还需 `7z`、`curl`、`zip`、`python3`）：

```bash
bash scripts/package-windows.sh
```

压缩包在 `dist/rust-mhxy-windows.zip`，解压整个目录后双击 `rust-mhxy.exe` 即可运行。Windows 程序使用 GUI 子系统，启动时不会弹出命令窗口；打包脚本会检查 EXE 的子系统标记。

单独编译的程序在 `target/x86_64-pc-windows-msvc/release/rust-mhxy.exe`。分发时需要一起带上：

- `assets/`（`maps.json` + `maps/` + 可选的内置 `models/`）
- `config.example.json`
- 运行库：`DirectML.dll`、`msvcp140*.dll`、`vcruntime140*.dll` 等（可 app-local 放在 exe 旁边）

本地 OCR 模型想离线可用，把 `pp-ocrv5_mobile_det.onnx` / `pp-ocrv5_mobile_rec.onnx` / `ppocrv5_dict.txt` 放到 `assets/models/` 即可。

## 目录结构

```
src/
  main.rs      # eframe 启动 + 中文字体加载
  app.rs       # egui 界面与识别调度
  theme.rs     # 全局配色、任务卡片和状态组件
  core.rs      # 任务解析、地图几何（移植自 Python core.py）
  capture.rs   # 窗口枚举与截图（Windows 用 Win32，其他用 xcap）
  gate.rs      # 图色门控：点位颜色匹配
  ocr.rs       # 本地 PP-OCR 工作线程 + 远端 PP-OCR 请求
  config.rs    # config.json 读写（支持注释）
assets/
  maps.json    # 只保留抓鬼/鬼王相关地图
  maps/*.png   # 地图图片
  models/      # 可选：随包携带的本地 PP-OCR 模型（缺省时自动下载）
config.example.json
```

## 配置

把 `config.example.json` 复制成 `config.json` 放在程序（或 `cargo run` 时的项目根目录）下。

| 键 | 说明 |
| --- | --- |
| `ocr/mode` | `local` 本地 PP-OCR；`remote` 远端 PP-OCR |
| `ocr/interval_seconds` | 识别间隔，1~60 秒 |
| `ocr/task_region` | 主识别区域 `左,上,右,下`（相对客户区百分比 0~1） |
| `ocr/extra_task_region` | 备用识别区域 |
| `ocr/extra_region_enabled` | 是否启用备用区域 |
| `api/format` | 远端请求格式：`multipart` / `raw` |
| `api/url` | PP-OCR 服务根地址 |
| `api/token` | Bearer Token，可留空 |
| `task/option/鬼王` | 是否识别鬼王 |
| `task/high_chance` | 是否叠加捉鬼高概率区域 |
| `task/horizontal_layout` | 任务布局：`false` 上下等分 / `true` 左右等分 |
| `color_gate/enabled` | 是否启用图色门控 |
| `color_gate/interval_ms` | 图色轮询间隔（毫秒，默认 300） |
| `color_gate/mode` | 多点逻辑：`all` 全部命中 / `any` 任一命中 |
| `color_gate/tolerance` | 新增点位的默认容差（每通道 0~255） |
| `color_gate/points` | 点位数组，每项 `{x, y, color, tolerance, radius, label}` |
| `log/level` | 日志级别：`simple` / `detailed` |
| `window/sidebar_expanded` | 左侧设置栏是否展开 |
| `window/scale` | 界面缩放：`0` 跟随系统 DPI，`0.75`~`2.5` 固定缩放（默认 `1.0`） |
| `window/coordinate/on_top` | 窗口是否置顶 |

界面里修改设置会在停止操作约 400ms 后自动回写 `config.json`，关闭程序时也会保存待写入的设置；保存失败会在界面提示。

## 界面

- 统一深色界面，抓鬼 / 鬼王使用青绿、金色区分，地图名与坐标突出显示；点击坐标可复制地图与坐标。
- 顶部「收起设置 / 展开设置」折叠侧栏；通过「开始识别 / 暂停识别」按钮控制识别，运行状态始终可见。
- 游戏窗口会自动选中（优先「梦幻西游 / MHXY」），也可以用下拉框手动选择、点「刷新」重扫。当前选中窗口可用时保留选择；识别中或尚未找到窗口时每 3 秒重扫，排除程序自己的窗口。
- 左侧栏按游戏窗口、识别方式、任务显示、界面、日志分组，整栏一起滚动。日志始终开启，简单模式记录启动与任务开始/停止，详细模式额外记录 OCR 过程和错误；仅绘制可见行，长日志悬停可查看全文。
- 远端连接测试结果直接显示在按钮下方，简单日志模式下也可查看成功或失败原因。
- 「设置识别区域」在**独立新窗口**里显示主/备用范围与图色门控，打开时更新一次截图，也可点击「更新截图」。设置区可滚动，预览始终保留空间；范围各字段标明左 / 上 / 右 / 下。
- 预览中红框表示主区域、蓝框表示备用区域，点击图片可取点取色；截图更新时复用纹理。
- **图色门控**默认每 300ms 轮询点位：命中允许 OCR，从未命中转为命中时立即重新识别；战斗或任务完成导致未命中时暂停 OCR。区域设置窗口打开时也会更新命中状态。
- 任务默认上下等分，可切换左右等分；关闭鬼王后抓鬼占满地图区。任务卡片标题与状态同一行左右排列：左侧显示任务、地图和坐标，右侧显示「等待识别」「本轮未识别到任务」或更新时间。标题行高度固定，状态变化不会改变地图尺寸；未识别到任务时保留上次地图和坐标。
- 切换游戏窗口会清除旧坐标；暂停、切换窗口或更改识别设置后，迟到的 OCR 结果会被丢弃。
- 底部状态栏显示识别方式、间隔、最近一次耗时和设置保存状态。

开发验证使用 `cargo test`、`cargo clippy --all-targets -- -D warnings` 和 `cargo run`；需要本地模型或真实任务截图的两项 OCR 集成测试默认忽略。

## 说明

- 地图图片按文件内容嗅探格式（仓库里的地图文件其实是 JPEG 但扩展名是 `.png`），不依赖扩展名。
- 本地 OCR 首次使用若无内置模型会自动下载到 `~/.oar/`。

## 与 Python 版的差异

- 只支持抓鬼 / 鬼王，任务模式与打宝图、降妖伏魔相关逻辑已移除
- 远端只保留 PP-OCR（移除了 mhxy-api 任务识别接口）
- 不支持多开窗口同时识别，每次只识别一个选中的游戏窗口
- 识别区域预览与设置放在独立的新窗口里
- 本地 OCR 模型为 PP-OCRv5（Python 版依赖 RapidOCR 自带模型）
