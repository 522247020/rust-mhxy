//! 键盘按键映射：把源按键替换成目标按键。
//!
//! Windows 下用低级键盘钩子（WH_KEYBOARD_LL）在按键送到前台程序之前拦截，
//! 再用 SendInput 补发目标按键；其它平台只保留配置与解析，不安装钩子。
use crate::config::KeyMapping;
use crate::hotkey::{self, Key};

/// 一条解析成功、可以直接装进钩子的映射。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyRule {
    pub from: Key,
    pub to: Key,
}

impl KeyRule {
    pub fn label(&self) -> String {
        format!(
            "{} → {}",
            hotkey::key_label(self.from),
            hotkey::key_label(self.to)
        )
    }
}

/// 解析界面 / 配置里的单条映射。源按键和目标按键都只能是单个按键。
pub fn parse_rule(mapping: &KeyMapping) -> Result<KeyRule, String> {
    let from = mapping.from.trim();
    let to = mapping.to.trim();
    if from.is_empty() {
        return Err("缺少源按键".to_string());
    }
    if to.is_empty() {
        return Err("缺少目标按键".to_string());
    }
    let from = hotkey::parse_key(from).map_err(|err| format!("源按键：{err}"))?;
    let to = hotkey::parse_key(to).map_err(|err| format!("目标按键：{err}"))?;
    if from == to {
        return Err("源按键和目标按键相同".to_string());
    }
    Ok(KeyRule { from, to })
}

/// 收集所有启用且能解析的映射。同一个源按键只取第一条，避免钩子里的匹配歧义。
pub fn collect_rules(mappings: &[KeyMapping]) -> Vec<KeyRule> {
    let mut rules: Vec<KeyRule> = Vec::new();
    for mapping in mappings.iter().filter(|mapping| mapping.enabled) {
        let Ok(rule) = parse_rule(mapping) else {
            continue;
        };
        if rules.iter().any(|existing| existing.from == rule.from) {
            continue;
        }
        rules.push(rule);
    }
    rules
}

/// 监听器用的签名：只有生效的规则变化才需要重建键盘钩子。
pub fn rules_signature(rules: &[KeyRule]) -> String {
    rules
        .iter()
        .map(KeyRule::label)
        .collect::<Vec<_>>()
        .join(", ")
}

/// 全局按键映射监听器。Drop 时通知钩子线程退出。
pub struct KeymapListener {
    #[cfg(windows)]
    thread_id: u32,
    handle: std::thread::JoinHandle<()>,
}

impl KeymapListener {
    /// 钩子线程是否还活着。线程意外退出时调用方应当重建。
    pub fn is_alive(&self) -> bool {
        !self.handle.is_finished()
    }
}

/// 安装全局键盘钩子（目前仅 Windows）。
pub fn spawn(rules: Vec<KeyRule>) -> Result<KeymapListener, String> {
    if rules.is_empty() {
        return Err("没有可用的按键映射".to_string());
    }
    #[cfg(windows)]
    {
        windows_impl::spawn(rules)
    }
    #[cfg(not(windows))]
    {
        let _ = rules;
        Err("键盘按键映射目前仅支持 Windows".to_string())
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::{KeyRule, KeymapListener};
    use std::sync::mpsc::channel;
    use std::thread;
    use std::time::Duration;
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
        UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, PM_NOREMOVE, WH_KEYBOARD_LL,
        WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    /// 钩子回调里只需要两个虚拟键码。
    #[derive(Clone, Copy)]
    struct HookRule {
        from: u32,
        to: u32,
    }

    // 低级键盘钩子的回调是无状态函数指针，规则通过装载钩子的线程局部变量传入。
    thread_local! {
        static RULES: std::cell::RefCell<Vec<HookRule>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// 方向键、Insert/Delete/Home/End/PageUp/PageDown 是扩展键，
    /// 注入时不带 KEYEVENTF_EXTENDEDKEY 会被游戏识别成小键盘上的同名键。
    fn is_extended(vk: u32) -> bool {
        matches!(vk, 0x21..=0x28 | 0x2D | 0x2E)
    }

    /// 补发一次目标按键。用扫描码 + EXTENDEDKEY，兼容按扫描码读键盘的游戏。
    unsafe fn send_key(vk: u32, down: bool) {
        let scan = MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) as u16;
        let mut flags = KEYBD_EVENT_FLAGS(0);
        if !down {
            flags |= KEYEVENTF_KEYUP;
        }
        let (w_vk, w_scan) = if scan != 0 {
            flags |= KEYEVENTF_SCANCODE;
            if is_extended(vk) {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            (VIRTUAL_KEY(0), scan)
        } else {
            // 极少数虚拟键没有扫描码，退回虚拟键注入。
            (VIRTUAL_KEY(vk as u16), 0)
        };
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: w_vk,
                    wScan: w_scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }

    unsafe extern "system" fn ll_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == 0 {
            let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            // SendInput 注入的事件带 LLKHF_INJECTED：直接放行，否则 F9→Tab、Tab→F9
            // 这种互相映射会在钩子里来回触发。
            if !kb.flags.contains(LLKHF_INJECTED) {
                let msg = wparam.0 as u32;
                let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
                let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
                if down || up {
                    let target = RULES.with(|rules| {
                        rules
                            .borrow()
                            .iter()
                            .find(|rule| rule.from == kb.vkCode)
                            .map(|rule| rule.to)
                    });
                    if let Some(target) = target {
                        send_key(target, down);
                        // 吞掉源按键，前台程序只会收到目标按键。
                        return LRESULT(1);
                    }
                }
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }

    pub fn spawn(rules: Vec<KeyRule>) -> Result<KeymapListener, String> {
        let hook_rules: Vec<HookRule> = rules
            .iter()
            .map(|rule| HookRule {
                from: rule.from.vk(),
                to: rule.to.vk(),
            })
            .collect();
        let (ready_tx, ready_rx) = channel::<Result<u32, String>>();
        let handle = thread::spawn(move || unsafe {
            // 低级键盘钩子的回调会在安装它的线程里执行，所以必须先建好消息队列。
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
            RULES.with(|slot| *slot.borrow_mut() = hook_rules);

            let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_proc), None, 0) {
                Ok(hook) => hook,
                Err(err) => {
                    let _ = ready_tx.send(Err(format!("安装键盘钩子失败：{err}")));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(GetCurrentThreadId()));

            loop {
                let mut msg = MSG::default();
                let result = GetMessageW(&mut msg, None, 0, 0);
                if result.0 == 0 || result.0 == -1 {
                    break;
                }
            }
            let _ = UnhookWindowsHookEx(hook);
        });

        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(thread_id)) => Ok(KeymapListener { thread_id, handle }),
            Ok(Err(err)) => Err(err),
            Err(_) => Err("安装键盘钩子超时".to_string()),
        }
    }

    impl Drop for KeymapListener {
        fn drop(&mut self) {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(from: &str, to: &str, enabled: bool) -> KeyMapping {
        KeyMapping {
            from: from.to_string(),
            to: to.to_string(),
            enabled,
        }
    }

    #[test]
    fn default_mappings_are_f9_and_tab_swapped() {
        let rules = collect_rules(&crate::config::default_key_mappings());
        assert_eq!(
            rules,
            vec![
                KeyRule {
                    from: Key::F(9),
                    to: Key::Tab,
                },
                KeyRule {
                    from: Key::Tab,
                    to: Key::F(9),
                },
            ]
        );
        assert_eq!(rules_signature(&rules), "F9 → Tab, Tab → F9");
    }

    #[test]
    fn disabled_invalid_and_duplicate_mappings_are_skipped() {
        let rules = collect_rules(&[
            mapping("F9", "Tab", false),
            mapping("f8", "tab", true),
            mapping("F8", "Space", true),
            mapping("", "Enter", true),
            mapping("F7", "F7", true),
        ]);
        assert_eq!(
            rules,
            vec![KeyRule {
                from: Key::F(8),
                to: Key::Tab,
            }],
            "关闭的、重复的、源键为空的、源键等于目标键的映射都不应生效"
        );
        assert!(parse_rule(&mapping("F9", "Nope", true)).is_err());
    }

    /// 手动确认 Windows 低级键盘钩子能装起来（不会模拟按键）。
    #[cfg(windows)]
    #[test]
    #[ignore = "会安装全局键盘钩子，手动运行：cargo test --bin rust-mhxy -- --ignored keymap_hook"]
    fn keymap_hook_installs_and_stays_alive() {
        let listener =
            spawn(collect_rules(&crate::config::default_key_mappings())).expect("安装键盘钩子失败");
        assert!(listener.is_alive(), "钩子线程应当存活");
    }
}
