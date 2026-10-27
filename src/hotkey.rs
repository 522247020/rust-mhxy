//! 手动识别用的快捷键：解析、Windows 全局注册、窗口内回退。
use std::sync::mpsc::Sender;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// F1~F24
    F(u8),
    /// 字母或数字主键（大写）
    Char(char),
    Space,
    Enter,
    Tab,
    Escape,
    Backspace,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
    pub key: Key,
}

impl Hotkey {
    /// 解析形如 `F8`、`Ctrl+F8`、`Alt+Shift+1` 的写法。
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("快捷键为空".into());
        }
        let (mut ctrl, mut shift, mut alt, mut win) = (false, false, false, false);
        let mut key = None;
        for part in text.split('+') {
            let part = part.trim();
            if part.is_empty() {
                return Err(format!("快捷键格式无效：{text}"));
            }
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "shift" => shift = true,
                "alt" => alt = true,
                "win" | "super" | "meta" | "cmd" | "command" => win = true,
                _ => {
                    if key.is_some() {
                        return Err("只能有一个主键".into());
                    }
                    key = Some(parse_key(part)?);
                }
            }
        }
        let key = key.ok_or_else(|| "缺少主键".to_string())?;
        Ok(Hotkey {
            ctrl,
            shift,
            alt,
            win,
            key,
        })
    }

    pub fn display(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.win {
            parts.push("Win".to_string());
        }
        parts.push(key_name(self.key).to_string());
        parts.join("+")
    }

    /// 窗口聚焦时的回退匹配（非 Windows 或全局注册失败时使用）。
    pub fn matches_egui(&self, ctx: &egui::Context) -> bool {
        let Some(egui_key) = to_egui_key(self.key) else {
            return false;
        };
        ctx.input(|input| {
            input.key_pressed(egui_key)
                && input.modifiers.ctrl == self.ctrl
                && input.modifiers.shift == self.shift
                && input.modifiers.alt == self.alt
        })
    }
}

impl Key {
    #[cfg(windows)]
    pub fn vk(self) -> u32 {
        match self {
            Key::F(n) => 0x70 + u32::from(n) - 1,
            Key::Char(c) => c.to_ascii_uppercase() as u32,
            Key::Space => 0x20,
            Key::Enter => 0x0D,
            Key::Tab => 0x09,
            Key::Escape => 0x1B,
            Key::Backspace => 0x08,
            Key::Insert => 0x2D,
            Key::Delete => 0x2E,
            Key::Home => 0x24,
            Key::End => 0x23,
            Key::PageUp => 0x21,
            Key::PageDown => 0x22,
            Key::Left => 0x25,
            Key::Right => 0x27,
            Key::Up => 0x26,
            Key::Down => 0x28,
        }
    }
}

fn parse_key(text: &str) -> Result<Key, String> {
    let lower = text.to_ascii_lowercase();
    if let Some(number) = lower.strip_prefix('f') {
        if let Ok(index) = number.parse::<u8>() {
            if (1..=24).contains(&index) {
                return Ok(Key::F(index));
            }
        }
    }
    if text.chars().count() == 1 {
        let ch = text.chars().next().unwrap();
        if ch.is_ascii_alphanumeric() {
            return Ok(Key::Char(ch.to_ascii_uppercase()));
        }
    }
    Ok(match lower.as_str() {
        "space" => Key::Space,
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "esc" | "escape" => Key::Escape,
        "backspace" => Key::Backspace,
        "insert" | "ins" => Key::Insert,
        "delete" | "del" => Key::Delete,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        _ => return Err(format!("不支持的按键：{text}")),
    })
}

fn key_name(key: Key) -> String {
    match key {
        Key::F(n) => format!("F{n}"),
        Key::Char(c) => c.to_string(),
        Key::Space => "Space".into(),
        Key::Enter => "Enter".into(),
        Key::Tab => "Tab".into(),
        Key::Escape => "Escape".into(),
        Key::Backspace => "Backspace".into(),
        Key::Insert => "Insert".into(),
        Key::Delete => "Delete".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::PageUp => "PageUp".into(),
        Key::PageDown => "PageDown".into(),
        Key::Left => "Left".into(),
        Key::Right => "Right".into(),
        Key::Up => "Up".into(),
        Key::Down => "Down".into(),
    }
}

fn to_egui_key(key: Key) -> Option<egui::Key> {
    use egui::Key as K;
    Some(match key {
        Key::F(n) => match n {
            1 => K::F1,
            2 => K::F2,
            3 => K::F3,
            4 => K::F4,
            5 => K::F5,
            6 => K::F6,
            7 => K::F7,
            8 => K::F8,
            9 => K::F9,
            10 => K::F10,
            11 => K::F11,
            12 => K::F12,
            13 => K::F13,
            14 => K::F14,
            15 => K::F15,
            16 => K::F16,
            17 => K::F17,
            18 => K::F18,
            19 => K::F19,
            20 => K::F20,
            21 => K::F21,
            22 => K::F22,
            23 => K::F23,
            24 => K::F24,
            _ => return None,
        },
        Key::Char(c) => match c.to_ascii_uppercase() {
            'A' => K::A,
            'B' => K::B,
            'C' => K::C,
            'D' => K::D,
            'E' => K::E,
            'F' => K::F,
            'G' => K::G,
            'H' => K::H,
            'I' => K::I,
            'J' => K::J,
            'K' => K::K,
            'L' => K::L,
            'M' => K::M,
            'N' => K::N,
            'O' => K::O,
            'P' => K::P,
            'Q' => K::Q,
            'R' => K::R,
            'S' => K::S,
            'T' => K::T,
            'U' => K::U,
            'V' => K::V,
            'W' => K::W,
            'X' => K::X,
            'Y' => K::Y,
            'Z' => K::Z,
            '0' => K::Num0,
            '1' => K::Num1,
            '2' => K::Num2,
            '3' => K::Num3,
            '4' => K::Num4,
            '5' => K::Num5,
            '6' => K::Num6,
            '7' => K::Num7,
            '8' => K::Num8,
            '9' => K::Num9,
            _ => return None,
        },
        Key::Space => K::Space,
        Key::Enter => K::Enter,
        Key::Tab => K::Tab,
        Key::Escape => K::Escape,
        Key::Backspace => K::Backspace,
        Key::Insert => K::Insert,
        Key::Delete => K::Delete,
        Key::Home => K::Home,
        Key::End => K::End,
        Key::PageUp => K::PageUp,
        Key::PageDown => K::PageDown,
        Key::Left => K::ArrowLeft,
        Key::Right => K::ArrowRight,
        Key::Up => K::ArrowUp,
        Key::Down => K::ArrowDown,
    })
}

fn egui_key_name(key: egui::Key) -> Option<&'static str> {
    use egui::Key as K;
    Some(match key {
        K::F1 => "F1",
        K::F2 => "F2",
        K::F3 => "F3",
        K::F4 => "F4",
        K::F5 => "F5",
        K::F6 => "F6",
        K::F7 => "F7",
        K::F8 => "F8",
        K::F9 => "F9",
        K::F10 => "F10",
        K::F11 => "F11",
        K::F12 => "F12",
        K::F13 => "F13",
        K::F14 => "F14",
        K::F15 => "F15",
        K::F16 => "F16",
        K::F17 => "F17",
        K::F18 => "F18",
        K::F19 => "F19",
        K::F20 => "F20",
        K::F21 => "F21",
        K::F22 => "F22",
        K::F23 => "F23",
        K::F24 => "F24",
        K::A => "A",
        K::B => "B",
        K::C => "C",
        K::D => "D",
        K::E => "E",
        K::F => "F",
        K::G => "G",
        K::H => "H",
        K::I => "I",
        K::J => "J",
        K::K => "K",
        K::L => "L",
        K::M => "M",
        K::N => "N",
        K::O => "O",
        K::P => "P",
        K::Q => "Q",
        K::R => "R",
        K::S => "S",
        K::T => "T",
        K::U => "U",
        K::V => "V",
        K::W => "W",
        K::X => "X",
        K::Y => "Y",
        K::Z => "Z",
        K::Num0 => "0",
        K::Num1 => "1",
        K::Num2 => "2",
        K::Num3 => "3",
        K::Num4 => "4",
        K::Num5 => "5",
        K::Num6 => "6",
        K::Num7 => "7",
        K::Num8 => "8",
        K::Num9 => "9",
        K::Space => "Space",
        K::Enter => "Enter",
        K::Tab => "Tab",
        K::Escape => "Escape",
        K::Backspace => "Backspace",
        K::Insert => "Insert",
        K::Delete => "Delete",
        K::Home => "Home",
        K::End => "End",
        K::PageUp => "PageUp",
        K::PageDown => "PageDown",
        K::ArrowLeft => "Left",
        K::ArrowRight => "Right",
        K::ArrowUp => "Up",
        K::ArrowDown => "Down",
        _ => return None,
    })
}

/// 录制快捷键：返回 egui 里刚按下的组合键，例如 `Ctrl+F8`。
pub fn capture_egui(ctx: &egui::Context) -> Option<String> {
    let mut captured = None;
    ctx.input(|input| {
        for event in &input.events {
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            {
                let Some(name) = egui_key_name(*key) else {
                    continue;
                };
                let mut parts = Vec::new();
                if modifiers.ctrl {
                    parts.push("Ctrl".to_string());
                }
                if modifiers.shift {
                    parts.push("Shift".to_string());
                }
                if modifiers.alt {
                    parts.push("Alt".to_string());
                }
                parts.push(name.to_string());
                captured = Some(parts.join("+"));
                break;
            }
        }
    });
    captured
}

/// 全局快捷键监听器。Drop 时通知线程退出并注销快捷键。
pub struct GlobalHotkey {
    #[cfg(windows)]
    thread_id: u32,
    #[allow(dead_code)]
    handle: std::thread::JoinHandle<()>,
    /// 注册时发现该组合已被其它程序占用（会提示用户换键）。
    /// 有键盘钩子兜底，所以即便这里非空，快捷键仍然能触发。
    pub warning: Option<String>,
}

impl GlobalHotkey {
    /// 监听线程是否还活着。
    ///
    /// 线程一旦退出，系统会自动注销它注册的热键，但界面上的状态仍然停在
    /// 「已注册」—— 这正是「提示已注册、按键却没反应」的来源。调用方发现
    /// 这里返回 `false` 时应当丢弃旧监听器并重新注册。
    pub fn is_alive(&self) -> bool {
        !self.handle.is_finished()
    }
}

/// 注册全局快捷键（目前仅 Windows）。成功时按下快捷键会向 `sender` 发送事件。
///
/// `id` 用来区分同一进程里的多个快捷键：WM_HOTKEY 只带 id，
/// 每个监听线程各自用不同的 id，避免以后改成共享消息队列时互相覆盖。
pub fn spawn(hotkey: Hotkey, id: i32, sender: Sender<()>) -> Result<GlobalHotkey, String> {
    #[cfg(windows)]
    {
        windows_impl::spawn(hotkey, id, sender)
    }
    #[cfg(not(windows))]
    {
        let _ = (hotkey, id, sender);
        Err("全局快捷键目前仅支持 Windows".to_string())
    }
}

/// 探测某个快捷键当前是否可用（未被其它程序占用）。非 Windows 恒为 true。
pub fn is_available(hotkey: Hotkey) -> bool {
    #[cfg(windows)]
    {
        windows_impl::is_available(hotkey)
    }
    #[cfg(not(windows))]
    {
        let _ = hotkey;
        true
    }
}

/// `desired` 被占用时，实测并返回一个空闲的替代组合。
/// 优先给常用、基本不会和游戏/输入法冲突的 F 键，其次才是加修饰键的变体。
pub fn suggest_available(desired: Hotkey) -> Option<Hotkey> {
    let mut candidates: Vec<Hotkey> = Vec::new();
    // 1) 常用 F 键
    for f in [8u8, 9, 6, 7, 10, 12, 5, 4] {
        candidates.push(Hotkey {
            ctrl: false,
            shift: false,
            alt: false,
            win: false,
            key: Key::F(f),
        });
    }
    // 2) Alt + F 变体
    for f in [8u8, 9, 10, 12] {
        candidates.push(Hotkey {
            ctrl: false,
            shift: false,
            alt: true,
            win: false,
            key: Key::F(f),
        });
    }
    // 3) 在原组合上补修饰键，尽量保留原来的主键手感
    candidates.push(Hotkey {
        ctrl: true,
        alt: true,
        ..desired
    });
    candidates.push(Hotkey {
        ctrl: true,
        shift: true,
        ..desired
    });
    candidates.push(Hotkey {
        alt: true,
        shift: true,
        ..desired
    });
    candidates
        .into_iter()
        .find(|candidate| *candidate != desired && is_available(*candidate))
}

#[cfg(windows)]
mod windows_impl {
    use super::{GlobalHotkey, Hotkey};
    use std::sync::mpsc::{channel, Sender};
    use std::thread;
    use std::time::Duration;
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL,
        MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN,
        VK_SHIFT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
        UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE, WH_KEYBOARD_LL, WM_HOTKEY,
        WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    /// 要监听的目标组合（供低级键盘钩子使用）。
    #[derive(Clone, Copy)]
    struct Desired {
        vk: u32,
        ctrl: bool,
        shift: bool,
        alt: bool,
        win: bool,
    }

    /// 监听线程启动时上报给调用方的信息。
    struct Ready {
        thread_id: u32,
        warning: Option<String>,
    }

    // 低级键盘钩子的回调是一个无状态函数指针，只能通过线程局部变量把配置和发送端传进去。
    // 钩子回调就在装载它的监听线程里执行，所以 thread_local 能正确解析到该线程的状态。
    thread_local! {
        static DESIRED: std::cell::Cell<Option<Desired>> = std::cell::Cell::new(None);
        static SENDER: std::cell::RefCell<Option<Sender<()>>> = std::cell::RefCell::new(None);
        static HOLDING_VK: std::cell::Cell<u32> = std::cell::Cell::new(0);
        static LAST_FIRE: std::cell::Cell<Option<std::time::Instant>> = std::cell::Cell::new(None);
    }

    fn desired_of(hotkey: Hotkey) -> Desired {
        Desired {
            vk: hotkey.key.vk(),
            ctrl: hotkey.ctrl,
            shift: hotkey.shift,
            alt: hotkey.alt,
            win: hotkey.win,
        }
    }

    fn modifier_bits(hotkey: Hotkey) -> u32 {
        // MOD_NOREPEAT：按住不放时不重复触发。
        let mut bits = MOD_NOREPEAT.0;
        if hotkey.ctrl {
            bits |= MOD_CONTROL.0;
        }
        if hotkey.shift {
            bits |= MOD_SHIFT.0;
        }
        if hotkey.alt {
            bits |= MOD_ALT.0;
        }
        if hotkey.win {
            bits |= MOD_WIN.0;
        }
        bits
    }

    fn key_down(vk: VIRTUAL_KEY) -> bool {
        unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
    }

    fn modifiers_match(desired: Desired) -> bool {
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        let alt = key_down(VK_MENU);
        let win = key_down(VK_LWIN) || key_down(VK_RWIN);
        ctrl == desired.ctrl && shift == desired.shift && alt == desired.alt && win == desired.win
    }

    /// 发送一次触发；WM_HOTKEY 与键盘钩子会对同一次按键都上报，这里做去重。
    fn fire() {
        let now = std::time::Instant::now();
        let skip = LAST_FIRE.with(|slot| {
            let skip = slot
                .get()
                .is_some_and(|last| now.duration_since(last) < Duration::from_millis(150));
            if !skip {
                slot.set(Some(now));
            }
            skip
        });
        if skip {
            return;
        }
        let _ = SENDER.try_with(|slot| {
            if let Some(tx) = slot.borrow().as_ref() {
                let _ = tx.send(());
            }
        });
    }

    /// 低级键盘钩子回调：在按键送到前台（含全屏游戏）之前就能看到，
    /// 即便游戏独占键盘、或系统热键被别的程序占用，也能触发。
    unsafe extern "system" fn ll_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == 0 {
            // HC_ACTION == 0：wParam/lParam 含有效按键信息。
            let desired = match DESIRED.try_with(|slot| slot.get()) {
                Ok(Some(desired)) => desired,
                _ => return CallNextHookEx(None, code, wparam, lparam),
            };
            let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            if desired.vk != 0 && kb.vkCode == desired.vk {
                let msg = wparam.0 as u32;
                let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
                let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
                if down {
                    let holding = HOLDING_VK.try_with(|cell| cell.get()).unwrap_or(0);
                    if holding != desired.vk && modifiers_match(desired) {
                        let _ = HOLDING_VK.try_with(|cell| cell.set(desired.vk));
                        fire();
                    }
                } else if up {
                    let _ = HOLDING_VK.try_with(|cell| {
                        if cell.get() == desired.vk {
                            cell.set(0);
                        }
                    });
                }
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }

    /// 探测组合是否空闲（未被其它程序占用）。
    pub fn is_available(hotkey: Hotkey) -> bool {
        unsafe {
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
            const PROBE_ID: i32 = 0x5151;
            let ok = RegisterHotKey(
                None,
                PROBE_ID,
                HOT_KEY_MODIFIERS(modifier_bits(hotkey)),
                hotkey.key.vk(),
            )
            .is_ok();
            if ok {
                let _ = UnregisterHotKey(None, PROBE_ID);
            }
            ok
        }
    }

    pub fn spawn(hotkey: Hotkey, id: i32, sender: Sender<()>) -> Result<GlobalHotkey, String> {
        let (ready_tx, ready_rx) = channel::<Result<Ready, String>>();
        let handle = thread::spawn(move || unsafe {
            // 先创建线程消息队列，RegisterHotKey(NULL) 的 WM_HOTKEY 会投递到这里。
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);

            DESIRED.with(|slot| slot.set(Some(desired_of(hotkey))));
            SENDER.with(|slot| *slot.borrow_mut() = Some(sender));

            // 低级键盘钩子先装：即便后面 RegisterHotKey 被占用失败，钩子也能兜底触发。
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_proc), None, 0).ok();

            // 尽量再注册系统全局热键：成功后系统会替我们“吃掉”这个组合，
            // 避免同一按键再传给其它程序重复触发；失败也不致命（钩子兜底）。
            let registered = RegisterHotKey(
                None,
                id,
                HOT_KEY_MODIFIERS(modifier_bits(hotkey)),
                hotkey.key.vk(),
            )
            .is_ok();
            let warning = if registered {
                None
            } else {
                Some(format!(
                    "{} 已被其他程序占用，可能同时触发对方；建议换一个组合",
                    hotkey.display()
                ))
            };

            if hook.is_none() && !registered {
                let _ = ready_tx.send(Err(format!(
                    "注册全局快捷键失败：{}（键盘钩子也不可用）",
                    hotkey.display()
                )));
                return;
            }

            let thread_id = GetCurrentThreadId();
            let _ = ready_tx.send(Ok(Ready { thread_id, warning }));

            loop {
                let mut msg = MSG::default();
                let result = GetMessageW(&mut msg, None, 0, 0);
                if result.0 == 0 || result.0 == -1 {
                    break;
                }
                if msg.message == WM_HOTKEY {
                    fire();
                }
            }
            if let Some(hook) = hook {
                let _ = UnhookWindowsHookEx(hook);
            }
            let _ = UnregisterHotKey(None, id);
        });

        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(ready)) => Ok(GlobalHotkey {
                thread_id: ready.thread_id,
                handle,
                warning: ready.warning,
            }),
            Ok(Err(err)) => Err(err),
            Err(_) => Err("注册全局快捷键超时".to_string()),
        }
    }

    impl Drop for GlobalHotkey {
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

    /// 回归测试：全局快捷键必须真的被系统持有，且消息能转发给调用方。
    ///
    /// 「提示已注册、按键却无效」这类问题只有这三段都成立才不会发生：
    /// 注册成功 → 系统持有 → 消息泵把 WM_HOTKEY 转出来。
    #[cfg(windows)]
    #[test]
    #[ignore = "会占用一个全局快捷键，手动运行：cargo test --bin rust-mhxy -- --ignored global_hotkey"]
    fn global_hotkey_is_held_by_system_and_forwards_messages() {
        use std::sync::mpsc::channel;
        use std::time::Duration;
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, MOD_ALT, MOD_CONTROL};
        use windows::Win32::UI::WindowsAndMessaging::{
            PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_HOTKEY,
        };

        let hotkey = Hotkey::parse("Ctrl+Alt+F24").unwrap();
        let (tx, rx) = channel::<()>();
        let Ok(listener) = spawn(hotkey, 98, tx) else {
            eprintln!("注册失败，跳过（可能已被其他程序占用）");
            return;
        };
        assert!(listener.is_alive(), "监听线程应当存活");

        // 同一个组合键只能被一个线程注册：注册线程若意外退出，这里就会成功。
        let taken = std::thread::spawn(|| unsafe {
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
            RegisterHotKey(None, 555, MOD_CONTROL | MOD_ALT, Key::F(24).vk()).is_err()
        });
        assert!(
            taken.join().unwrap(),
            "同一个组合键被成功注册了两次，说明第一次注册没有生效"
        );

        unsafe { PostThreadMessageW(listener.thread_id, WM_HOTKEY, WPARAM(98), LPARAM(0)) }
            .expect("投递 WM_HOTKEY 失败");
        assert!(
            rx.recv_timeout(Duration::from_secs(2)).is_ok(),
            "消息泵没有把 WM_HOTKEY 转发给调用方"
        );
    }

    #[test]
    fn parses_and_displays_hotkeys() {
        let hotkey = Hotkey::parse("Ctrl+Shift+F8").unwrap();
        assert!(hotkey.ctrl && hotkey.shift && !hotkey.alt);
        assert_eq!(hotkey.key, Key::F(8));
        assert_eq!(hotkey.display(), "Ctrl+Shift+F8");

        let single = Hotkey::parse("f9").unwrap();
        assert!(!single.ctrl && single.key == Key::F(9));
        assert_eq!(single.display(), "F9");

        assert!(Hotkey::parse("Ctrl+").is_err());
        assert!(Hotkey::parse("").is_err());
        assert!(Hotkey::parse("Ctrl+Unknown").is_err());
    }

}
