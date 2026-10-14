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
- 本地识图：内置 **PP-OCRv5**（oar-ocr + ONNX Runtime），优先用随包 `assets/models` 里的模型，找不到时自动下载
- 远端识图：把截图 PNG 直接发给 PP-OCR 服务，支持 `multipart`（`/ocr`）和 `raw`（`/ocr/raw`）
- 解析「抓鬼 / 捉鬼 / 鬼王」文字，匹配地图与坐标
- 地图上标出坐标点、附近范围（`附近` 关键字）、捉鬼高概率区域（50×50）
- 配置写入 `config.json`（支持注释，兼容 Python 版字段）

## 构建与运行

Windows 是主要目标平台（窗口客户区截图用 Win32 API 精确实现）。

```bash
cd rust-mhxy
cargo run --release
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

产物在 `target/x86_64-pc-windows-msvc/release/rust-mhxy.exe`。分发时需要一起带上：

- `assets/`（`maps.json` + `maps/` + 可选的内置 `models/`）
- `config.example.json`
- 运行库：`DirectML.dll`、`msvcp140*.dll`、`vcruntime140*.dll` 等（可 app-local 放在 exe 旁边）

本地 OCR 模型想离线可用，把 `pp-ocrv5_mobile_det.onnx` / `pp-ocrv5_mobile_rec.onnx` / `ppocrv5_dict.txt` 放到 `assets/models/` 即可。

## 目录结构

```
src/
  main.rs      # eframe 启动 + 中文字体加载
  app.rs       # egui 界面与识别调度
  core.rs      # 任务解析、地图几何（移植自 Python core.py）
  capture.rs   # 窗口枚举与截图（Windows 用 Win32，其他用 xcap）
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
| `log/enabled` | 是否在左侧边栏底部输出带时间的日志 |
| `window/sidebar_expanded` | 左侧设置栏是否展开 |
| `window/coordinate/on_top` | 窗口是否置顶 |

界面里修改设置会自动回写 `config.json`。

## 界面

- 顶部「隐藏侧栏 / 显示侧栏」按钮可以折叠左侧设置栏
- 游戏窗口会自动选中（优先「梦幻西游 / MHXY」），也可以用下拉框手动选、点「刷新」重扫；每 3 秒后台重扫一次，不会覆盖你手动选的窗口
- 左侧设置栏底部是**日志框**（默认关闭，勾选「开启日志」后输出带时间的任务情况），可拖动上边缘调整高度
- 「显示识别区域」勾选或点「打开识别区域设置…」会在**独立新窗口**里打开：上半部是主/备用区域设置，下半部是实时截图预览（红框主区域、蓝框备用区域）；窗内「手动截图一次」按钮可以只截屏刷新预览、不做识别
- 抓鬼 / 鬼王任务在中间区域等分显示：默认上下等分，可切换左右等分；关闭鬼王后抓鬼占满整个区域

## 与 Python 版的差异

- 只支持抓鬼 / 鬼王，任务模式与打宝图、降妖伏魔相关逻辑已移除
- 远端只保留 PP-OCR（移除了 mhxy-api 任务识别接口）
- 不支持多开窗口同时识别，每次只识别一个选中的游戏窗口
- 识别区域预览与设置放在独立的新窗口里
- 本地 OCR 模型为 PP-OCRv5（Python 版依赖 RapidOCR 自带模型）
