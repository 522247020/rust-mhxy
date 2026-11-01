# 点阵字库（原型）

大漠插件那种「字库」思路的 Rust 实现，用来替代 / 校验 OCR 对**坐标数字**的识别。
分支：`feat/ocr-font-lib`，代码：`src/font_lib.rs`（库）+ `src/bin/font_tool.rs`（工具）。

## 为什么是闭集匹配

坐标数字只有 12 类（`0-9`、`,`、`.`），地图名只有几十个。闭集只需要 argmax，
不需要绝对精度，所以在小字号、低对比度、抗锯齿下比通用 OCR 稳得多；代价是
必须**建库**，字体/DPI 变了要重建。

## 流程

```
RGB → 灰度 → Otsu 二值化（黑字白底）
     → 竖直投影切字（列空隙 ≤ gap_merge 视为同一个字）
     → 用「整行行高」统一裁剪每个字（不是各字墨迹外框，否则逗号/句点会归一化成同一个墨团）
     → 缩放到固定 cell（默认 16x24）
     → zNCC 打分，±shift 像素平移取最优
     → argmax + margin 判定；分数不够高时尝试从中间劈开（粘连字）
```

建库和识别共用同一套切字 / 归一化代码，所以字模和运行时输入同分布。

## 用法

### 0. 先把识别区域对上（位置不对时）

```bash
# 看游戏窗口的客户区坐标
cargo run --bin font-tool -- windows

# 整屏截图 + 拖拽框选：自动算出相对客户区的百分比，并当场拍一张校验图
cargo run --bin font-tool -- pick --write-config
#   Enter 确认、Esc 取消；--write-config 会把结果写回 config.json
#   （ocr/task_region + 当前分辨率的 region_presets）
```

`pick` 会做三件事：

1. 从拖拽的屏幕坐标反算相对客户区的百分比（自动选「与选区重叠最多」的窗口，
   不依赖窗口枚举顺序）；
2. 检查选区中心是不是被别的窗口盖住（盖住了会提醒）；
3. 按算出的区域真截一张图，和框选画面比平均像素差——一致才算调对。

> **屏幕缩放**：`font-tool` 启动时会把自己设成 per-monitor-v2 DPI 感知，
> 和主程序（egui/winit）一致。否则系统开 125%/150% 时两边的客户区尺寸会差一个
> 缩放倍数（比如 809x632 对 1213x948），区域和 `region_presets` 全对不上。
> `windows` 命令首行会打印 `屏幕 / 系统缩放 / DPI 感知`，先看它。

不开界面直接写区域：

```bash
cargo run --bin font-tool -- region --region 0.795,0.292,0.985,0.538 --size 1218x994
cargo run --bin font-tool -- region --region 0.795,0.292,0.985,0.538 --dry-run  # 只打印
```

只需要单张截图（不跑 OCR）时用：

```bash
cargo run --bin font-tool -- snap --out target/panel.png            # 当前区域
cargo run --bin font-tool -- snap --desktop --out target/desktop.png # 整屏
cargo run --bin font-tool -- crop --image target/desktop.png --crop 931,739,812,663 --out x.png
```

### 1. 自动采集（推荐）

不用裁剪、不用手打标注：工具自己截当前游戏窗口、跑本地 PP-OCR 拿文字、
按检测框切成字模、把 OCR 文字当标签，多帧**聚类 + 投票**后直接写字库。

```bash
# 先看一眼区域对不对（只截图，不跑 OCR）
cargo run --bin font-tool -- snap --out target/panel.png

# 采集 20 帧，直接产出字库
cargo run --bin font-tool -- capture --out assets/fontlib.json --frames 20 --interval 800
```

窗口、区域、预处理、放大倍数默认取 `config.json` 里的现有设置（包括按分辨率
保存的 `ocr/region_presets`），也可以用参数覆盖：

```bash
cargo run --bin font-tool -- capture --out assets/fontlib.json \
  --window 梦幻西游 --region 0.70,0.24,0.99,0.70 \
  --preprocess color,binary --color #eaff65 --tolerance 24 --upscale 2
```

采集时每个字模样本都会投票：

- 按外观聚类（相似度 ≥ `--min-similarity`，默认 0.9）——同字体同抗锯齿的字模几乎一样；
- 同一个簇内对标签投票，平票保持原判，只有**严格多数**才改判；
- OCR 认错的单帧会被压掉；如果一整个簇多数票都是错的，说明 OCR 在这一段系统性
  认错，会用「提醒」打出来，让你判断要不要重建。

默认只学数字和分隔符（`0-9`、`,，、.．。`），避免把界面上的中文和噪声塞进坐标字库；
`--all` 连中文一起学（做地图名字库时才需要）。

参数：`--frames` 帧数、`--interval` 间隔毫秒、`--cell 16x24`、`--dump <目录>`
（把每帧面板和各检测框的裁剪图存下来，方便排查采错）、`--min-similarity`、
`--padding`（裁剪框外扩像素，默认 2，和运行时复核一致）。

输出示例：

```
窗口：梦幻西游（客户区 Some((812, 663))）｜区域 (0.795, 0.292, 0.985, 0.538)（来自 config.json 的 ocr/task_region）｜预处理 关闭｜放大 2x｜20 帧，间隔 800 ms
第 1/20 帧：面板 308x326，采到 12 个字模｜OCR 抓鬼 长寿村 79,26 附近 / 鬼王 天宫 12,45
...
已写入 assets/fontlib.json（16x24，12 个字模，164 个样本，来自 18/20 帧）
,×1（12 票）  .×1（6 票）  0×1（14 票）  ...
```

### 2. 手动准备样本（可选）

如果自动采集的标签不可信，也可以给准确标注：

```
samples/
  line1.png   # 一行文字截图（原始截图或已二值化都行）
  line1.txt   # 这行的准确文字，例如：79,26
  line2.png
  line2.txt
```

```bash
cargo run --bin font-tool -- build --samples samples --out fontlib.json
```

输出里会列出每个字的样本数。切出来的字数与 `.txt` 字数不一致的样本会被跳过并
打警告（通常是截图裁掉了字）。

### 3. 在截图上读字

```bash
# 整块面板：先按行切分
cargo run --bin font-tool -- read --lib fontlib.json --image panel.png

# 整张图就是一行
cargo run --bin font-tool -- read --lib fontlib.json --image line.png --line

# 从截图里裁一块再看，顺便把二值化结果 dump 出来核对切字
cargo run --bin font-tool -- read --lib fontlib.json --image shot.png \
  --crop 200,300,260,80 --save-binary target/debug-binary.png
```

每行输出 `y=<行顶> <识别文本> [不可信] 每个字(label/分数/margin)`。

### 4. 真实截图冒烟测试

```bash
MHXY_FONT_SAMPLES=samples MHXY_FONT_IMAGE=panel.png \
  cargo test --bin font-tool reads_real_screenshot -- --ignored --nocapture
```

支持 `MHXY_FONT_CROP=x,y,w,h`。

## 参数（`MatchConfig`）

| 字段 | 默认 | 作用 |
|---|---|---|
| `min_ink` | 1 | 一列至少多少黑像素算笔画 |
| `gap_merge` | 1 | 列空隙 ≤ 该值并成同一个字 |
| `split_when_below` | 0.95 | 分数低于该值才尝试劈开 |
| `split_gain` | 0.05 | 两半都要比整块高这么多分才接受拆分 |
| `max_split_depth` | 3 | 递归拆分深度上限 |
| `min_split_width` | 4 | 窄于该像素数的字不尝试拆分 |
| `shift` | 2 | 匹配允许的 ±像素平移 |
| `min_score` | 0.5 | 低于该分数判不可信 |
| `min_margin` | 0.05 | 与次优 label 的差值过低判不可信（形近字） |
| `row_gap` | 2 | 行切分的空隙容差 |

## 接入 OCR 流程

主程序里已经接上，开关默认关：侧边栏「识别方式」卡片 →「点阵字库复核坐标」。
对应配置：

```json5
"ocr/font_lib": true,
"ocr/font_lib_path": "fontlib.json",  // 留空 = assets/fontlib.json，相对路径相对 assets
```

流程（`src/coord_fix.rs`）：

```
OCR 返回「文本 + 每个检测框的坐标 / 置信度 / 字符区间」
  → 在框内文本里找坐标形状 d{1,3}[,，、.。]d{1,3}
  → 按框坐标从「送 OCR 的那张图」里裁图（外扑 2px），字库重读
  → 只接受：字库高置信 且 整块刚好就是一个坐标 且 字数与 OCR 框内一致
  → 用字符区间就地替换文本里的坐标，其余文字一个字都不动
  → 文本继续走 parse_tasks + 地图边界校验
```

几个细节：

- 只对**本地识别**生效：远端接口只回纯文本，没有检测框，无法定位要复核哪一块。
- OCR 把数字认成字母（`6→G`、`0→O`、`1→l`）时，替换区间会吞掉这些字母，但不会吞中文
  和全角标点，所以「坐标（79,26）附近」这种结构不会被拆错。
- 字库推翻 OCR 时会在详细日志里留一行 `字库复核坐标 79,2G → 79,26`，方便核对
  是字库救回来了还是字库改坏了。
- 字库读不确信（分数 / margin 不够、形状对不上、字数对不上）就**完全不改**，行为退回改造前。
- 「取点窗口 → 测试 OCR」故意不复核，看到的永远是 OCR 原文，便于分清是谁认错了。

## 已知限制

- 字体/字号/DPI 变化大时要重建库；`cell` 尺寸固定，极端长宽比会失真。
- 形近字（`0/O`、`1/l`、`8/B`）靠 `margin` 暴露风险，不保证分辨。
- 纯色块（空白、全黑）zNCC 无意义，直接判 0 分并标记不可信。
- 目前只做「认数字」：建库用数字/标点样本即可；地图名仍靠 OCR + 模糊匹配。

## 采集后怎么判断够用

```bash
cargo run --bin font-tool -- info --lib assets/fontlib.json
```

```
assets/fontlib.json：16x24，24 个字模
0×3  1×7  2×8  5×1  7×5
提醒：还缺这些坐标字符：34689,.
```

**必须覆盖 `0-9` 和 `,` `.`**，缺一个就意味着含该数字的坐标永远判「不可信」→
不替换，等于没生效。缺字符通常是因为采集时任务栏里没有带坐标的任务：
抓鬼/鬼王/降妖这类「去 XX（79,26）附近」的任务行才会贡献数字。

采集会**自动合并**到同一份字库（`--fresh` 才从零重建），所以可以边做任务边跑，
分几次把 12 个字符补全。每次都会打印缺什么。
