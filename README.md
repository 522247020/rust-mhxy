# rust-mhxy

梦幻西游抓鬼 / 鬼王 / 通用任务坐标辅助工具。识别任务栏中的地图与坐标，在内置的 20 张地图上标出目标位置和「附近」范围。

基于 **Rust + egui** 开发，支持本地 PP-OCRv5 和远端 PP-OCR，提供 Windows 10 / 11 64 位桌面应用。

[![Windows build](https://github.com/522247020/rust-mhxy/actions/workflows/windows.yml/badge.svg)](https://github.com/522247020/rust-mhxy/actions/workflows/windows.yml)
[![Release](https://img.shields.io/github/v/release/522247020/rust-mhxy)](https://github.com/522247020/rust-mhxy/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## 界面预览

主界面：左侧是基础设置（游戏窗口、识别模式、任务类型、显示与布局），底部固定「开始识别 / 暂停识别」；顶栏右侧是高级设置和打坐入口。

![主界面](docs/screenshots/main.png)

精简界面只保留地图，进入时底部会提示「按 Esc 退出精简界面」：

| 精简界面 | 进入时的退出提示 |
| --- | --- |
| ![精简界面](docs/screenshots/compact.png) | ![精简模式提示](docs/screenshots/compact-hint.png) |

高级设置和打坐计时各自是独立窗口：

| 高级设置 | 打坐计时 |
| --- | --- |
| ![高级设置](docs/screenshots/advanced.png) | ![打坐计时](docs/screenshots/meditation.png) |

## 快速开始

第一次使用按下面 8 步走即可；默认设置已经能用，不需要先改配置表。

### 1. 下载并解压

从 [xyq.mhwy.work](https://xyq.mhwy.work) 或 [GitHub Releases](https://github.com/522247020/rust-mhxy/releases/latest) 下载 Windows 64 位压缩包。

- 解压到**可写目录**，不要直接在压缩包里运行。
- 保留 `assets` 目录和压缩包内的所有 DLL。
- Windows 分发包已包含本地 OCR 模型、地图、DirectML 和 VC++ 运行库，本地识别可离线使用。

### 2. 启动程序

双击 `rust-mhxy.exe`，程序不会弹出命令行窗口。

首次启动的默认状态：

- 本地 PP-OCRv5、自动识别、抓鬼 + 鬼王。
- 主识别区域为游戏客户区宽度的 `70%～99%`、高度的 `24%～70%`。
- 设置会自动保存到 exe 同目录的 `config.json`；从源码运行时保存在项目根目录。

首次本地识别会加载模型；缺少模型时会自动下载约 25 MB。没有本地模型又暂时不想下载时，可以在顶栏「高级设置 → 识别方式」切换到远端 PP-OCR。

### 3. 选择游戏窗口

打开游戏，并保持游戏窗口可见、任务栏区域没有被其他窗口遮挡。

1. 看左侧「游戏窗口」下拉框，程序会自动扫描并优先列出标题含「梦幻西游 / MHXY」的窗口。
2. 没找到就点「刷新」；仍未找到时确认游戏没有最小化。
3. 可以手动切换窗口。切换后会清空上一窗口的任务结果。

选中后，下拉框下方会显示当前客户区分辨率，以及这个分辨率是否已经记住识别区域。

### 4. 选择任务

在左侧「任务类型」里勾选需要的面板：

| 选项 | 说明 |
| --- | --- |
| 抓鬼任务 | 解析抓鬼 / 捉鬼任务。 |
| 鬼王任务 | 解析鬼王任务，单独显示一块地图面板。 |
| 通用任务（只看地图名和坐标） | 不判断任务类型，只按「地图名 + 坐标」标注。与抓鬼 / 鬼王互斥，勾选后会自动启用备用区域。 |

- 至少保留一个任务，否则不会解析任务。
- 高概率区域和上下 / 左右布局在基础区「显示与布局」里调整；只勾选一个任务时，面板会占满地图区域。
- 高概率区域按 175LG 省眼规则叠加 50×50 格子，并且只画在黄色「附近」范围框内。

### 5. 选择自动或手动

在左侧「识别模式」中选择：

| 模式 | 启动后行为 | 启动按钮 |
| --- | --- | --- |
| 自动（按间隔） | 按「识别间隔」定时截图识别，间隔范围 1～60 秒。 | 开始识别 |
| 手动（快捷键） | 不主动定时识别；按一次快捷键识别一次，也可以点「立即识别一次」。 | 开始监听 |

两种模式都需要先点侧栏底部的「开始识别 / 开始监听」进入运行状态。关于快捷键：

- 手动模式默认快捷键是 `F8`，可以在输入框直接填写，也可以点「录制」后按一次组合键；录制时按 `Esc` 取消。
- 自动模式运行中也可以按快捷键立即识别一次，不必先切到手动模式。
- 快捷键触发的识别会忽略图色门控和「画面未变化」去重；识别进行中按下的快捷键会排队，在本轮结束后补跑。

快捷键的权限、冲突和回退规则见[手动快捷键与常见故障](#手动快捷键与常见故障)。

### 6. 开始识别

点侧栏底部的「开始识别」（自动）或「开始监听」（手动）。运行中按钮会变成「暂停识别 / 暂停监听」；侧栏滚动时这个主按钮始终固定可见。

- 如果侧栏已收起，先点左上角的「展开设置 / 设置」，再点底部的启动按钮。
- 抓鬼面板显示目标红点、黄色「附近」范围；高概率区域可选。
- 本轮没有识别到任务时，面板会保留上一次的结果，不会突然清空。
- 自动模式只在任务栏画面变化时重新 OCR；手动触发不受这条限制。
- 想让窗口压在游戏上面，点顶部「置顶」；只想看地图，点「精简界面」，进入后底部会提示按 `Esc` 退出。

### 7. 调整识别区域

如果地图一直不更新，先检查识别区域，而不是先改一堆 OCR 参数。

1. 左侧「游戏窗口 → 设置识别区域」打开独立窗口，点「更新截图」。
2. 在右侧预览中确认红框是否框住任务文字；蓝框是备用区域。
3. 在「截图范围」中调整主区域的左、上、右、下百分比，或启用并调整备用区域。修改即时生效。
4. 点「测试 OCR」看当前截图和预处理下的识别文字、解析结果与耗时。
5. 确认文字完整后回到主窗口继续识别。

默认区域通常适用于 1920×1080 等常见布局。任务文字跨到第二块区域时启用备用区域；勾选「通用任务」会自动启用。

识别区域修改后会自动保存，不需要额外保存操作。分辨率记忆规则见[按分辨率记住识别区域](#按分辨率记住识别区域)。

### 8. 识别不对时

先把顶栏「高级设置」里的日志级别设为「详细」（写入 `logs/rust-mhxy.log`），再按下面的顺序排查：

| 详细日志 / 现象 | 先检查 | 处理 |
| --- | --- | --- |
| `OCR 没有识别到文字` | 红框是否框住任务文字、游戏是否最小化或被遮挡 | 更新截图并调整识别区域；确认游戏窗口仍在 |
| 有 `OCR 文字：…`，但显示「本轮未识别到任务」 | 勾选的任务类型、文字是否完整 | 补勾正确任务；用「测试 OCR」确认文字没有缺字 |
| `图色未命中，暂停 OCR` | 图色门控是否启用、点位是否还命中 | 临时关闭门控，或重新取点 / 调整容差与半径 |
| `识别区域未变化，跳过` | 这是自动模式的去重，不是报错 | 等任务栏发生变化，或按快捷键 / 「立即识别一次」强制识别 |
| 文字是白字黑底、坐标容易缺位 | 预处理效果 | 在识别区域窗口勾选「二值化」，必要时再加「反色」 |
| 远端识别失败 | 服务地址、Token、请求格式 | 在顶栏「高级设置 → 识别方式」点「测试连接」；本地模式不需要服务 |
| 换分辨率后区域不对 | 分辨率是否已被记住 | 看侧栏的「当前分辨率」提示，切回该分辨率后重新识别 |

识别区域窗口顶部的「测试 OCR」不会影响正式识别，是调区域和预处理最快的办法。

## 界面说明

### 顶栏

顶栏从左到右：检查更新、说明、界面主题、置顶、精简界面、高级设置、打坐；开始 / 暂停固定在侧栏底部，窗口再矮也不用滚动去找。

| 按钮 | 作用 |
| --- | --- |
| 检查更新 | 手动检查更新；发现新版本时按钮变金色，点击查看说明并下载。 |
| 说明 | 打开程序内的使用说明。 |
| 界面主题 | 跟随系统 / 亮色 / 暗色，默认跟随系统。 |
| 置顶 | 让本窗口显示在游戏前面。 |
| 精简界面 | 隐藏顶栏、状态栏和设置栏，只留地图；进入时底部提示「按 Esc 退出」，按 `Esc` 恢复。 |
| 高级设置 | 打开独立设置窗口：识别方式、方案管理、合成旗标注。 |
| 打坐 | 打开打坐计时设置窗口：启用开关、时长、悬浮窗样式和快捷键。 |

窗口变窄时，「更多」菜单会收进检查更新、说明、主题、置顶、精简界面、高级设置和打坐；开始 / 暂停仍在侧栏底部。

### 基础设置（侧栏默认显示）

| 分组 | 内容 |
| --- | --- |
| 游戏窗口 | 选择 / 刷新窗口；设置识别区域；当前分辨率与记忆状态；「忘记」当前分辨率。 |
| 识别模式 | 自动（按间隔）/ 手动（快捷键）；自动模式的识别间隔；手动模式的快捷键、录制、立即识别一次。 |
| 任务类型 | 抓鬼、鬼王（可同时勾选）；通用任务是独立模式，勾选后会关闭另外两个。 |
| 显示与布局 | 抓鬼 / 鬼王的高概率区域开关；上下 / 左右布局。 |

侧栏底部固定显示「开始识别 / 开始监听」，下方是当前窗口、OCR 方式和触发摘要。底栏显示上次耗时、配置保存状态，以及「发布页 / GitHub」链接。

### 高级设置窗口（顶栏按钮）

点顶栏「高级设置」打开独立窗口，日常使用时不用打开。识别区域和门控只在基础区「设置识别区域」里调整，这里不重复出现：

| 分组 | 内容 |
| --- | --- |
| 识别方式 | 本地 PP-OCR / 远端 PP-OCR；远端服务地址、Token、请求格式和「测试连接」。 |
| 方案管理 | 识别方案的下拉切换、保存、另存为、删除；方案包含识别区域、图色门控和预处理。 |
| 合成旗标注 | 打开独立的合成旗编辑器，维护各大地图的飞行点。 |

打坐计时有自己的顶栏入口，见[打坐倒计时](#打坐倒计时)。

### 日志

日志不再显示在界面里，直接写入磁盘：

- `config.json` 同目录下的 `logs/rust-mhxy.log`
- 文件超过 2 MB 时自动轮转成 `rust-mhxy.log.1`（只保留一份旧日志）
- 「简单」只记启动、任务开始 / 停止、错误；「详细」额外记录 OCR 原文、解析结果和失败原因

「设置识别区域」窗口里还有更细的调整：

- **识别方案**：保存 / 切换识别区域 + 图色门控 + 预处理组合（顶栏「高级设置 → 方案管理」也能操作）。
- **截图范围**：主区域、备用区域。
- **图色门控设置**：点位、颜色、容差、半径、命中逻辑。
- **识别预处理**：颜色提取、灰度、二值化、反色。

### 识别区域窗口

窗口采用左右布局：

- 顶部工具条：识别方案、更新截图、测试 OCR、点击预览取点。
- 上排左侧：截图范围、图色门控设置；右侧：识别区域预览（红框主区域、蓝框备用区域）。
- 下排左侧：识别预处理；右侧：主区域 / 备用区域的处理预览，左右并排对比。

点击预览图可以取图色点位，也可以取文字颜色。取点模式由工具条的「点击预览取点」和预处理里的「点击预览取色」控制。

#### 识别方案

「识别方案」保存的是**图色门控 + 预处理设置**，不包含截图区域和游戏窗口。

- 下拉切换方案，立即生效。
- 「保存」把当前设置写回当前方案；「另存为」新建方案；「删除」删除当前方案（至少保留一个）。
- 方案名后有 `*` 表示有未保存的修改。

#### 图色门控

图色门控用少量像素判断任务栏当前是否可识别，适合在战斗或任务完成时跳过 OCR。

1. 在识别区域窗口点「更新截图」。
2. 勾选「点击预览取点」，在预览中点击任务栏的稳定特征点（通常是任务图标或固定文字颜色）。
3. 视需要调整每个点的容差、半径和删除点位。
4. 勾选「启用」，选择「全部命中」或「任一命中」。

状态会直接显示「有任务：允许 OCR」或「未命中：暂停 OCR」。没有配置点位时默认允许 OCR；手动触发会忽略门控。

#### 识别预处理

任务栏背景的花纹、动画和彩色底图会干扰 OCR。预处理在送 OCR 前处理截图，四个步骤可以任意组合，按固定顺序执行：

| 步骤 | 说明 |
| --- | --- |
| 颜色提取 | 只保留指定文字颜色附近的像素，输出黑字白底。适合文字颜色稳定的任务栏。 |
| 灰度 | 去掉颜色干扰，保留笔画。 |
| 二值化 | Otsu 自动阈值，输出白底黑字。日常首选。 |
| 反色 | 最后把黑白颠倒；二值化把文字判成白色时才需要。 |

常见组合：只选「二值化」；背景花且文字颜色稳定时用「颜色提取 + 二值化」；白字黑底时加「反色」。

主区域和备用区域分别设置、分别处理，再拼接送一次 OCR。选「颜色提取」时可以：

- 在「色值」里填 `#RRGGBB`，默认 `#eaff65`（RGB `234, 255, 101`），点「默认」恢复。
- 点「点击预览取色」后在右侧预览里点任务文字。
- 用「用图色点位颜色」复用图色门控的取点。

拿不准时先点「测试 OCR」，看预览里的文字是否完整、黑白是否颠倒，再回去调。

## 识别方式

在顶栏「高级设置 → 识别方式」中切换。

同一处还有三个提高识别精度的选项：

| 选项 | 作用 |
| --- | --- |
| 放大倍数 | 送 OCR 前把截图放大 1~3 倍。任务栏文字较小时检测模型容易漏字，放大后召回明显提高 |
| 高召回检测 | 把 PP-OCR 检测阈值从 0.3/0.6 降到 0.2/0.4，并放宽文字框外扩；宁可多几个噪声框也不漏字 |
| 多帧投票 | 地图取最近几帧众数、坐标取同一地图最近 3 帧的中位数；新地图连续出现 2 帧才切换，避免单帧错认来回跳 |

| 方式 | 使用说明 |
| --- | --- |
| 本地 PP-OCR | 使用 PP-OCRv5，首次识别时加载模型。优先读取 `assets/models/`，缺少模型时自动下载到 `~/.oar/`。 |
| 远端 PP-OCR | 填写服务地址和可选的 Bearer Token，点「测试连接」确认服务可用。截图以 PNG 格式发送。 |

远端服务支持两种格式：

- `multipart`：请求 `/ocr`，图片字段为 `file`。
- `raw`：请求 `/ocr/raw`，请求体为 PNG 图片字节。

服务地址填写根地址即可，例如 `http://192.168.1.100:8008`，程序会按请求格式补全路径。远端模式启动前必须填写地址，否则会提示先填写地址或切回本地。

## 手动快捷键与常见故障

手动识别快捷键默认是 `F8`，适用于：

- 手动（快捷键）模式：点「开始监听」后，每次按键识别一次。
- 自动（按间隔）模式：运行中按键也会立即识别一次。

Windows 下通过系统全局快捷键注册，游戏窗口在前台也能触发。侧栏会显示注册状态：

- `全局快捷键已注册`：可用，游戏在前台也能触发。
- `开始监听后生效`：还没有进入运行状态，点侧栏底部的启动按钮即可。
- `正在注册…`：正在注册，稍等；失败后每 5 秒重试。
- 显示 Windows 返回的错误：通常是快捷键已被其他程序占用，换一个组合键。

已知检查和限制：

- **必须先启动识别**。没有点「开始识别 / 开始监听」时，快捷键不会触发。
- 游戏以管理员身份运行时，本程序也可能需要以管理员身份运行。
- 全局快捷键目前仅 Windows 支持。非 Windows 或全局注册失败时，退回到「本窗口聚焦时可用」，并写入详细日志。
- 默认打坐快捷键是 `F9`，与识别快捷键独立；如果和其他软件冲突，换成带 `Ctrl` / `Alt` 的组合更稳妥。
- 识别进行中按下的快捷键会排队补跑，不会直接丢失。

## 按分辨率记住识别区域

不同游戏分辨率下任务栏位置不同。程序按**游戏客户区尺寸**（例如 `1920x1080`）记住主识别区域：

1. 识别时检查客户区尺寸；发现分辨率变化后，把当前主区域保存到上一个分辨率。
2. 尝试套用新分辨率保存过的区域。
3. 新分辨率没有记录时，先沿用当前区域；调整后，下次切走时就会记住。
4. 切回同一分辨率时自动套用，不需要重新调整。

注意事项：

- 记忆发生在**识别时**。暂停期间拖动窗口改变大小，不会立刻切换；下一次识别才会处理。
- 目前只按分辨率记住**主区域**；备用区域是统一配置，不随分辨率切换。
- 侧栏会显示「当前分辨率 1920x1080」和「已记住该分辨率的区域 / 首次使用，调整后会自动记住」。
- 想恢复默认，点当前分辨率的「忘记」；之后该分辨率使用默认主区域，可以重新调整。
- 配置分别保存在 `ocr/region_presets`（各分辨率区域）和 `ocr/task_region`（当前区域）中。

## 打坐倒计时

点顶栏「打坐」打开设置窗口，勾选「启用打坐计时」后，屏幕上出现一个无边框透明悬浮窗。

- 左键单击悬浮窗：开始 / 暂停 / 继续；拖动可移动位置。
- 全局快捷键（默认 `F9`）：开始 / 重新计时；可填写或录制修改。
- 「时长」按分钟调整，范围 1～180 分钟。
- 「悬浮窗置顶」让计时器压在其他窗口上面；「锁定」后整窗鼠标穿透，只能在主界面取消。
- 背景不透明度为 0 时只剩数字；字号为 0 时跟随窗口高度自动缩放；可以选「加粗」。
- 到点后数字闪红几秒，不播放声音。

> 游戏以管理员身份运行时，全局快捷键可能收不到按键，请同样用管理员身份运行本程序，或换一个游戏没有占用的组合键。

## 合成旗标注

合成旗点位保存在本地，不进入安装包，也不会被自动更新覆盖。

1. 打开顶栏「高级设置 → 合成旗标注 → 打开合成旗编辑器」。
2. 选择地图（长安城、傲来国、长寿村、朱紫国），新建一面合成旗。
3. 选中合成旗后直接点击地图添加坐标；点位列表里的 `×` 可删除。
4. 每面旗可以单独设置背景色、文字色和标注大小；同一面旗的所有点显示相同的青色数字编号，地图上以数字圆点标注。

Windows 默认保存在 `%LOCALAPPDATA%\rust-mhxy\flight_flags.json`；Linux 默认保存在 `$XDG_CONFIG_HOME/rust-mhxy/flight_flags.json` 或 `~/.config/rust-mhxy/flight_flags.json`。开发和测试时可用 `MHXY_FLIGHT_FLAGS_PATH` 指定其他路径。

当前仓库缺少长安城地图资源，该地图暂时无法点击取点或绘制；补齐地图资源后即可使用。

## 配置

大部分设置可以直接在界面里改，程序会自动保存到 exe 同目录的 `config.json`，下次启动时恢复。开发运行时配置保存在项目根目录。

需要手动配置时，把 [config.example.json](config.example.json) 复制为 `config.json`。配置支持 `//` 和 `/* */` 注释，修改后重启程序生效。

### 识别与区域

| 配置项 | 说明 |
| --- | --- |
| `ocr/mode` | `local` 本地识别，`remote` 远端识别。 |
| `ocr/interval_seconds` | 自动识别间隔，1～60 秒。 |
| `ocr/trigger` | `auto` 按间隔自动识别；`manual` 快捷键手动触发。 |
| `ocr/upscale` | 送 OCR 前放大截图的倍数（1~3），默认 2。 |
| `ocr/high_recall` | 高召回检测：降低检测阈值减少漏字，默认开启。 |
| `ocr/vote_frames` | 多帧投票：地图取众数、坐标取中位数，默认开启。 |
| `ocr/hotkey` | 手动快捷键，如 `F8`、`Ctrl+F8`；Windows 下可全局生效。 |
| `ocr/task_region` | 主区域，格式 `左,上,右,下`，值为 0～1 的客户区百分比。 |
| `ocr/extra_task_region` | 备用区域，格式同主区域。 |
| `ocr/extra_region_enabled` | 是否启用备用区域。 |
| `ocr/region_presets` | 按分辨率保存的主区域，键如 `1920x1080`，值格式同主区域。 |

### OCR、预处理与方案

| 配置项 | 说明 |
| --- | --- |
| `ocr/preprocess` | 预处理管线，`off` 或逗号组合：`color`、`gray`、`binary`、`invert`，如 `binary`、`color,binary`。 |
| `ocr/preprocess_color` | 颜色提取保留的文字颜色，十六进制，默认 `#eaff65`。 |
| `ocr/preprocess_tolerance` | 颜色提取容差，0～255。 |
| `ocr/extra_preprocess` | 备用区域独立预处理管线；未写时跟随主区域。 |
| `ocr/extra_preprocess_color` | 备用区域颜色提取的文字颜色。 |
| `ocr/extra_preprocess_tolerance` | 备用区域颜色提取的容差。 |
| `api/url` | 远端 PP-OCR 服务根地址。 |
| `api/token` | Bearer Token，可留空。 |
| `api/format` | `multipart` 或 `raw`。 |
| `profiles/active` | 当前识别方案名称。 |
| `profiles/items` | 保存的识别方案数组（图色门控 + 预处理设置）。 |

### 任务与图色门控

| 配置项 | 说明 |
| --- | --- |
| `task/option/抓鬼` | 是否显示抓鬼面板。 |
| `task/option/鬼王` | 是否同时识别鬼王。 |
| `task/option/通用` | 通用任务；与抓鬼 / 鬼王互斥。 |
| `task/horizontal_layout` | `false` 上下排列，`true` 左右排列。 |
| `task/high_chance` | 抓鬼地图是否叠加高概率区域。 |
| `task/high_chance/鬼王` | 鬼王地图是否叠加高概率区域。 |
| `color_gate/enabled` | 是否启用图色门控。 |
| `color_gate/interval_ms` | 门控采样间隔，默认 300 毫秒，范围 50～5000。 |
| `color_gate/mode` | `all` 全部命中，`any` 任一命中。 |
| `color_gate/tolerance` | 新增点位的默认颜色容差。 |
| `color_gate/points` | 点位数组，支持坐标、颜色、容差、半径和标签。 |

### 界面、日志与更新

| 配置项 | 说明 |
| --- | --- |
| `log/level` | `simple` 简单日志，`detailed` 详细日志；日志写盘，不在界面展示。 |
| `window/sidebar_expanded` | 是否展开设置侧栏。 |
| `window/theme` | `system` 跟随系统（默认）/ `light` / `dark`。 |
| `window/sidebar/folded` | 侧栏折叠的分组 id，逗号分隔。 |
| `window/region/scope_open` | 识别区域窗口「截图范围」是否展开。 |
| `window/region/gate_open` | 识别区域窗口「图色门控」是否展开。 |
| `window/region/preprocess_open` | 识别区域窗口「识别预处理」是否展开。 |
| `window/coordinate/on_top` | 主窗口是否置顶。 |
| `update/enabled` | 启动后是否自动检查更新。 |
| `update/manifest_url` | 更新清单地址，默认 `https://xyq.mhwy.work/latest.json`。 |

### 打坐计时

| 配置项 | 说明 |
| --- | --- |
| `meditation/enabled` | 是否启用打坐倒计时（悬浮窗 + 全局快捷键）。 |
| `meditation/duration_seconds` | 倒计时时长（秒），范围 60～10800。 |
| `meditation/clock_format` | `false` 只显示秒数，`true` 显示 `分:秒`。 |
| `meditation/opacity` | 悬浮窗外框不透明度，0～1；0 为全透明。 |
| `meditation/font_px` | 字号（像素），0 表示跟随窗口高度。 |
| `meditation/text_color` | 时间文字颜色。 |
| `meditation/bold` | 是否使用粗体。 |
| `meditation/hotkey` | 开始 / 重新计时的全局快捷键。 |
| `meditation/on_top` | 悬浮窗是否置顶。 |
| `meditation/locked` | 是否锁定为鼠标穿透。 |
| `meditation/position` | 悬浮窗位置 `x,y`；留空由系统摆放。 |

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

安装包下载到 `%LOCALAPPDATA%\rust-mhxy\updates`（更新窗口里会显示完整路径），更新脚本的日志是同目录的 `update.log`。脚本会等旧进程退出、重试覆盖最多 10 秒，然后再启动新版本；如果程序装在 `Program Files` 这类需要管理员权限的目录，覆盖会失败，日志里会写明原因。

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

**更新说明的来源**：默认使用标签注释，例如 `git tag -a v0.1.7 -m "修复坐标抖动；新增自动更新"`；想单独维护时，在仓库放 `site/notes.txt`（一行一条，参考 [`site/notes.example.txt`](site/notes.example.txt)），工作流会优先使用它。

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

#### act_runner 持久化缓存（推荐）

#### runner 的 compose（本仓库维护者使用）

```yaml
services:
  runner:
    restart: always
    image: gitea/act_runner:latest
    environment:
      CONFIG_FILE: /config.yaml
      GITEA_INSTANCE_URL: "http://192.168.124.254:6600/"
      GITEA_RUNNER_REGISTRATION_TOKEN: "<token>"
      GITEA_RUNNER_NAME: "NAS"
      GITEA_RUNNER_LABELS: "${RUNNER_LABELS}"
    ports:
      - 8088:8088            # act_runner 自带 cache server（给 actions/cache 用）
    volumes:
      - /volume4/docker/app_config/act_runner/config.yaml:/config.yaml   # CONFIG_FILE 指的就是它
      - /volume4/docker/app_config/act_runner/data:/data                 # .runner 注册信息、日志
      - /var/run/docker.sock:/var/run/docker.sock                        # 起作业容器必需
```

这份 compose 不需要为缓存做任何改动：作业容器的 `/cache` 来自 `config.yaml` 的 `container.options`（宿主机路径），而不是 compose 的卷。**改了 `config.yaml` 一定要 `docker restart act_runner-runner-1`。**

act_runner 默认每个任务起一个全新容器：`/data` 是 runner 自己的目录，作业容器访问不到；`/workspace/<owner>/<repo>` 是**每任务一个命名卷**（名字里带 task id，任务结束即删除）；`$HOME` 也是全新的。所以这些位置都不能当缓存。

act_runner 会把内置命名卷 `act-toolcache` 挂到作业容器的 **`/opt/hostedtoolcache`**，它跨任务、跨仓库持久（宿主机路径 `/volume4/@docker/volumes/act-toolcache/_data`）。**工作流默认就用它**，不需要任何 runner 配置：

```
工具链缓存目录：/opt/hostedtoolcache/rust-mhxy
```

`CARGO_HOME`、`RUSTUP_HOME`、`CARGO_TARGET_DIR`、`XWIN_CACHE_DIR`、`ORT_CACHE_DIR`、`OAR_HOME` 都指向这个目录下的子目录。

### 可选：改用独立的 `/cache`

如果想把缓存和 toolcache 分开、或者放到更宽裕的磁盘上，可以给作业容器额外挂一个 `/cache`：

1. 宿主机建目录（和 runner 数据目录同级）：

```bash
sudo mkdir -p /volume4/docker/app_config/act_runner/cache
sudo chmod -R 777 /volume4/docker/app_config/act_runner/cache
```

2. 编辑 runner 的 `config.yaml`（compose 里挂到 `/config.yaml` 还是 `/data/config.yaml`，用第 3 步的命令确认）。**`container.options` 和 `valid_volumes` 必须同时配**，否则 act 的 `sanitizeConfig` 会把不在 `valid_volumes` 里的所有 bind 静默丢弃（runner 日志：`... is not a valid volume, will be ignored`）：

```yaml
container:
  options: "--volume /volume3/ssd-3-temp/runner:/cache"
  # 必须与上面 --volume 的源路径完全一致（glob 精确匹配），否则卷会被静默丢弃
  valid_volumes:
    - /volume3/ssd-3-temp/runner
```

3. 确认 runner 真正读的是哪个配置文件，再重启，并检查卷是否被丢弃：

```bash
docker inspect act_runner-runner-1 \
  --format 'image={{.Config.Image}} cmd={{.Config.Cmd}} env={{.Config.Env}} mounts={{json .Mounts}}'
docker exec act_runner-runner-1 sh -c 'grep -n -A6 "^container:" /config.yaml /data/config.yaml 2>/dev/null'

docker restart act_runner-runner-1

# 任务运行时，在宿主机上确认作业容器真的挂了 /cache
docker ps --filter ancestor=docker.gitea.com/runner-images:ubuntu-latest --format '{{.Names}}' |
while read -r n; do docker inspect "$n" --format '{{json .Mounts}}'; done
```

挂上以后作业日志会变成：

```
::notice::使用 /cache（container.options 挂载生效）
```

优先级：`MHXY_CACHE_DIR`（可用 `runner.envs` 指定）> `/cache` > `/opt/hostedtoolcache/rust-mhxy` > 工作区 `.ci-cache`（不持久，仅兜底）。`CARGO_TARGET_DIR` 会持续增长，定期清理 `cache/target` 或 `toolcache/rust-mhxy/target` 即可。

如果 runner 配置的是 **host 模式**（任务直接在宿主机跑），不需要挂载，`$HOME` 本身就是持久的，工作流会自动复用。

也可以在任何机器上手动部署已生成的站点：

```bash
bash scripts/package-windows.sh
python scripts/build-site.py --notes "本次更新内容"
SITE_SSH_HOST=xyq.mhwy.work SITE_SSH_USER=deploy SITE_PATH=/var/www/xyq.mhwy.work   SITE_SSH_KEY=~/.ssh/deploy_key bash scripts/deploy-site.sh dist/site
```

## 项目结构

```text
src/
  main.rs        # 应用启动、版本信息与中文字体
  app.rs         # 界面、任务状态与识别调度
  theme.rs       # 配色与通用界面组件
  core.rs        # 任务解析与地图坐标计算
  capture.rs     # 窗口枚举与截图
  gate.rs        # 图色门控
  ocr.rs         # 本地与远端 OCR
  preprocess.rs  # OCR 预处理管线
  hotkey.rs      # 快捷键解析与 Windows 全局注册
  meditation.rs  # 打坐倒计时状态与悬浮窗
  flight_flags.rs # 合成旗点位与本地存储
  config.rs      # 配置读写
  update.rs      # 自动更新：清单检查、下载校验与替换
assets/
  maps.json     # 地图定义
  maps/         # 地图图片
scripts/
  package-windows.py  # Windows 分发包组装
  build-site.py       # 生成静态下载页与更新清单
  package-windows.sh  # Linux 交叉编译入口
  deploy-site.sh      # 部署静态下载站
.github/workflows/
  windows.yml   # Windows 自动构建与发布
.gitea/workflows/
  site.yml      # Gitea 构建与部署
config.example.json
site/
  index.html    # 下载页模板（占位符由 build-site.py 填充）
  style.css     # 页面样式
  favicon.svg   # 站点图标
  notes.txt     # 发布说明，推 v* 标签时由下载站工作流读取
LICENSE
```

## 更新日志

完整版本历史和安装包见 [GitHub Releases](https://github.com/522247020/rust-mhxy/releases)。

### v0.2.12

- **代码结构整理**：`src/app.rs` 按功能拆成 `app/{mod,shell,sidebar,region,flight,meditation,task,update,tests}.rs`，最大的业务文件从 6800 行降到约 1200 行，行为未变。
- **配置解析迁移**：`config.json` 改用 `json5` + serde derive 解析，删掉约 450 行手写读取/保存样板；仍然支持 `//`、`/* */` 注释、尾逗号、旧键（`task/option/降妖伏魔`、`meditation/visible`）和旧主题值，保存格式与旧版字节级一致。
- **精简模式提示**：进入精简界面时，窗口底部显示 4 秒「按 Esc 退出精简界面」，避免找不到退出方式。
- README 新增「界面预览」：主界面、精简界面、精简提示、高级设置、打坐计时截图。

### v0.2.11

- **高级设置、打坐改为顶栏入口**：高级设置和打坐计时各自打开独立窗口，侧栏只保留常用项；识别区域和门控不再在高级设置里重复出现。
- **显示与布局回到基础区**：高概率区域开关和上下 / 左右布局放回侧栏，和任务类型放在一起。
- **识别方案包含识别区域**：方案现在同时保存主区域、备用区域和备用区域开关；切换方案立即套用，并同步到当前分辨率的区域记录。旧方案缺少区域字段时保持当前区域不变。
- **通用任务独立**：通用任务与抓鬼 / 鬼王互斥，勾选鬼王时也会自动取消通用；解析层增加兜底，即使配置里三个都开启也只用通用。
- **日志改为写盘**：不再在界面显示日志，直接写入 `config.json` 同目录的 `logs/rust-mhxy.log`，超过 2 MB 自动轮转为 `rust-mhxy.log.1`；日志级别仍在高级设置里切换。
- 修复打坐 / 高级设置窗口内容超出时没有滚动条、被窗口裁掉的问题；滚动区现在占满窗口高度。

### v0.2.10

- 修复手动识别快捷键不生效：运行中即注册全局快捷键，注册失败每 5 秒重试并在设置里显示原因。
- 自动模式下手动按键也会立即识别一次，不必先切模式。
- 识别进行中按下的快捷键会排队补跑，不再直接丢弃。
- 手动识别与打坐计时使用各自独立的快捷键 id，避免相互干扰。

## 开源协议

本项目采用 [MIT License](LICENSE) 开源协议。
