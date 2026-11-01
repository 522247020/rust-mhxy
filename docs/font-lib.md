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

### 1. 准备样本

```
samples/
  line1.png   # 一行文字截图（原始截图或已二值化都行）
  line1.txt   # 这行的准确文字，例如：79,26
  line2.png
  line2.txt
```

建议多准备几行、不同数字组合、不同字号/DPI 各一版；同一个字有多个样本时会
全部保留，匹配时取分数最高的那个。

### 2. 建库

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

## 已知限制

- 字体/字号/DPI 变化大时要重建库；`cell` 尺寸固定，极端长宽比会失真。
- 形近字（`0/O`、`1/l`、`8/B`）靠 `margin` 暴露风险，不保证分辨。
- 纯色块（空白、全黑）zNCC 无意义，直接判 0 分并标记不可信。
- 目前只做「认字」，还没接进 `ocr` 流程：下一步是把识别区间锚定到 OCR 给出的
  bbox，用字库结果覆盖 `find_coordinate` 的数字部分，再由 `core::parse_tasks`
  和 `coordinate_in_map` 做一致性校验。
