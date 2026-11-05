//! OCR 文本归一化：把识别结果整理成后续解析可以直接使用的形态。
//!
//! 主路径中的位置是「投影分行 → 每行 rec-only 识别 → **本模块** → 地图名/坐标解析」。
//! 这里只做纯文本变换：不读图、不写文件、不依赖 OCR 引擎，方便单独写单测。
//!
//! 规则取舍面向梦幻西游任务栏：
//! - 游戏字体里的数字和标点基本是半角，但 OCR 经常输出全角数字、全角逗号、
//!   句号或顿号，所以先统一成半角/逗号；
//! - 任务栏同时可能出现贴图图标、emoji、装饰符号，统一删掉；
//! - 只保留 CJK 基本区（U+4E00..=U+9FFF）和少量 ASCII 字符。地图名、任务名
//!   都在 CJK 基本区内；CJK 扩展区、假名、韩文等不属于当前游戏文本，直接丢弃；
//! - 数字串内部的空格没有语义（`79, 26`、`7 9` 都是同一个坐标），要删掉。
//!
//! 如果还需要纠正字母/数字混淆，建议先 [`snap_confusables`] 再 [`normalize`]：
//! `normalize` 的白名单里没有 `|`，先归一化会把可能代表数字 1 的竖线直接删掉。

/// 全角→半角、分隔符统一、去噪字符、空白规整。
///
/// 具体规则：
/// 1. 全角 ASCII（U+FF01..=U+FF5E）减 `0xFEE0` 转成半角；全角空格 U+3000
///    转成普通空格。
/// 2. `，`、`、`、`．`、`。` 统一成 `,`。其中 `．` 虽然也落在全角 ASCII
///    区间，但必须在通用转换前处理，否则会变成 `.`。
///    **取舍**：之所以把 `。` 也当逗号，是因为任务栏里的句号几乎不会表示小数点，
///    而 OCR 把坐标逗号认成句号很常见；坐标分隔符语义优先。ASCII `.` 保持原样，
///    与核心解析里的 `,`、`.` 两种分隔符一起接受。
/// 3. 括号/引号统一：`（）`→`()`、`【】`→`[]`、`「」`→`"`；`：`、`；`、`！`、
///    `？` 统一成对应的半角符号（它们本来也会被第 1 条转掉，显式列出便于阅读）。
///    `「」` 映射成 `"` 而不是删除，是为了保留“这里原本有引号”的结构；英文引号
///    不区分左右，成对出现时就是两个 `"`。
/// 4. 删除控制字符、emoji 以及其他不在允许集合内的字符。允许集合：CJK 基本区、
///    ASCII 数字、ASCII 字母，以及 ` ,.;:!?-()[]/"`。中文标点会在前面的规则里
///    先完成映射，未列出的中文标点（如 `《》`）会被删除。
/// 5. 空白规整：所有空白（空格、换行、制表符等）先视为空格，连续空白压成一个，
///    再去掉首尾空白。最后删除**数字串内部**的空格：空格两侧都是数字/坐标分隔符
///    且至少一侧是数字时删除，因此 `79, 26`→`79,26`、`7 9`→`79`，
///    而 `抓鬼 长寿村` 中的空格保留。
///
/// 函数是幂等的：`normalize(normalize(s)) == normalize(s)`。
pub fn normalize(text: &str) -> String {
    let mut filtered = String::with_capacity(text.len());
    for ch in text.chars() {
        let mapped = match ch {
            '\u{3000}' => ' ',
            // 必须在下面的全角区间之前：`．` 要变成逗号，不能变成点号。
            '，' | '、' | '．' | '。' => ',',
            '【' => '[',
            '】' => ']',
            '「' | '」' => '"',
            // 这几个本来就在 U+FF01..=U+FF5E 里，显式列出是为了让规则一目了然。
            '：' => ':',
            '；' => ';',
            '！' => '!',
            '？' => '?',
            '（' => '(',
            '）' => ')',
            '\u{FF01}'..='\u{FF5E}' => {
                // 区间内减 0xFEE0 一定落在 ASCII 可见字符范围，失败分支只为稳妥。
                char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch)
            }
            _ => ch,
        };

        if mapped.is_whitespace() {
            filtered.push(' ');
        } else if is_allowed(mapped) {
            filtered.push(mapped);
        }
    }

    // 先压空白再去首尾，避免 `trim` 后重新出现连续空白。
    let mut collapsed = String::with_capacity(filtered.len());
    let mut pending_space = false;
    for ch in filtered.chars() {
        if ch == ' ' {
            pending_space = true;
        } else {
            if pending_space && !collapsed.is_empty() {
                collapsed.push(' ');
            }
            pending_space = false;
            collapsed.push(ch);
        }
    }

    let chars: Vec<char> = collapsed.chars().collect();
    let mut out = String::with_capacity(collapsed.len());
    for (index, &ch) in chars.iter().enumerate() {
        if ch == ' ' && is_numeric_space(&chars, index) {
            continue;
        }
        out.push(ch);
    }
    out
}

/// 判断字符是否属于归一化后允许保留的集合。
///
/// 单独拆出来是为了让过滤规则只有一处，避免以后加白名单时漏改。
fn is_allowed(ch: char) -> bool {
    matches!(
        ch,
        '\u{4E00}'..='\u{9FFF}'
            | '0'..='9'
            | 'A'..='Z'
            | 'a'..='z'
            | ' '
            | ','
            | '.'
            | ';'
            | ':'
            | '!'
            | '?'
            | '-'
            | '('
            | ')'
            | '['
            | ']'
            | '/'
            | '"'
    )
}

/// 空格是否夹在数字串内部（两侧是数字或坐标分隔符，且至少一侧是数字）。
///
/// 这样 `79, 26` 里的空格会删掉，而 `抓鬼 长寿村` 里的空格会保留。
/// `chars` 已经压过空白，所以下标两侧不会再有空格。
fn is_numeric_space(chars: &[char], index: usize) -> bool {
    if index == 0 || index + 1 >= chars.len() {
        return false;
    }
    let prev = chars[index - 1];
    let next = chars[index + 1];
    let numeric = |ch: char| ch.is_ascii_digit() || ch == ',' || ch == '.';
    numeric(prev) && numeric(next) && (prev.is_ascii_digit() || next.is_ascii_digit())
}

/// 把 OCR 常混淆的字母映射成数字（只对字母生效，中文原样保留）。
///
/// 映射表：`O o D`→`0`、`I |`→`1`、`Z`→`2`、`A`→`4`、`S`→`5`、`G`→`6`、
/// `T`→`7`、`B`→`8`、`g q`→`9`。
///
/// `i` 和 `l` 只有在**紧邻原文本中的 ASCII 数字**时才映射成 `1`。它们与数字
/// 1 的相似度低于大写 `I`，脱离数字上下文时统一映射容易误伤正常字母；而只要
/// 邻居已经是数字，基本可以确定它们也是坐标数字被认错。判断只看原文本，
/// 不会把同一轮刚映射出来的数字再当作上下文。
///
/// 只映射表中明确列出的字符，不扩展大小写变体（例如小写 `s`、`z`、`d` 不动），
/// 避免把正常文本改坏。函数只做一对一替换，不增删字符、不处理空白。
///
/// 注意：`normalize` 的白名单里没有 `|`，如果 OCR 原文可能把 `1` 认成竖线，
/// 应先调用本函数再调用 [`normalize`]，否则竖线会在归一化时被删掉。
pub fn snap_confusables(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (index, &ch) in chars.iter().enumerate() {
        let prev_is_digit = index > 0 && chars[index - 1].is_ascii_digit();
        let next_is_digit = chars
            .get(index + 1)
            .is_some_and(|next| next.is_ascii_digit());
        let mapped = match ch {
            'O' | 'o' | 'D' => '0',
            'I' | '|' => '1',
            'i' | 'l' if prev_is_digit || next_is_digit => '1',
            'Z' => '2',
            'A' => '4',
            'S' => '5',
            'G' => '6',
            'T' => '7',
            'B' => '8',
            'g' | 'q' => '9',
            _ => ch,
        };
        out.push(mapped);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_maps_fullwidth_digits_and_comma() {
        assert_eq!(normalize("７９，２６"), "79,26");
    }

    #[test]
    fn normalize_keeps_parentheses_around_coordinate() {
        assert_eq!(
            normalize("去长寿镇（７９，２６）附近"),
            "去长寿镇(79,26)附近"
        );
        assert_eq!(normalize("【79，26】"), "[79,26]");
    }

    #[test]
    fn normalize_removes_spaces_inside_numbers() {
        assert_eq!(normalize("79, 26"), "79,26");
        assert_eq!(normalize("7 9"), "79");
        assert_eq!(normalize("7 9 , 2 6"), "79,26");
        assert_eq!(normalize("坐标 79, 26 附近"), "坐标 79,26 附近");
    }

    #[test]
    fn normalize_drops_emoji_and_noise() {
        let text = normalize("抓鬼 ★ 长寿村");
        assert!(!text.contains('★'));
        assert_eq!(text, "抓鬼 长寿村");
        assert_eq!(normalize("抓\u{0007}鬼🌀"), "抓鬼");
    }

    #[test]
    fn normalize_unifies_punctuation() {
        assert_eq!(normalize("79、26。3．4，5"), "79,26,3,4,5");
        assert_eq!(normalize("【79,26】「任务」"), "[79,26]\"任务\"");
        assert_eq!(
            normalize("坐标：79,26；提示！真的？"),
            "坐标:79,26;提示!真的?"
        );
    }

    #[test]
    fn normalize_collapses_whitespace_and_trims() {
        assert_eq!(normalize("  抓鬼\t\n长寿村  "), "抓鬼 长寿村");
    }

    #[test]
    fn normalize_is_idempotent() {
        let samples = [
            "７９，２６",
            "去长寿镇（７９，２６）附近",
            "79, 26",
            "7 9",
            "抓鬼 ★ 长寿村",
            "  抓鬼\t\n长寿村  ",
            "【79,26】「任务」：抓鬼！",
            "７ ９ 、 ２ ６ 。 附 近",
        ];
        for sample in samples {
            let once = normalize(sample);
            assert_eq!(
                normalize(&once),
                once,
                "样本 {sample:?} 未幂等，第一次归一化结果：{once:?}"
            );
        }
    }

    #[test]
    fn snap_confusables_maps_coordinate_letters() {
        assert_eq!(snap_confusables("79,2G"), "79,26");
        assert_eq!(snap_confusables("OoDI|ZASTBgq"), "000112457899");
    }

    #[test]
    fn snap_confusables_maps_i_and_l_only_next_to_digits() {
        assert_eq!(snap_confusables("l2,OQ"), "12,0Q");
        assert_eq!(snap_confusables("i2"), "12");
        assert_eq!(snap_confusables("2l"), "21");
        assert_eq!(snap_confusables("i"), "i");
        assert_eq!(snap_confusables("l"), "l");
    }

    #[test]
    fn snap_confusables_keeps_chinese() {
        assert_eq!(snap_confusables("抓鬼"), "抓鬼");
        assert_eq!(snap_confusables("长寿村 7O,2G"), "长寿村 70,26");
    }
}
