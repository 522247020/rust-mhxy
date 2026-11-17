//! 全局快捷键 / 键盘钩子的「按需注册」状态机。
//!
//! OCR 快捷键、按键映射、打坐快捷键三处要的东西完全一样：算出「想要注册什么」→
//! 和上次尝试的比较 → 探活 → 变更/重试两种限流 → 重建监听器 → 只在第一次失败时记日志。
//! 以前三份各写一遍，细节已经开始分叉（有的清 error、有的不清），这里收成一份。
use super::*;
use crate::hotkey::GlobalHotkey;
use crate::keymap::KeymapListener;

/// 快捷键文本变化后的注册节流，避免输入过程中每个字符注册一次。
pub(super) const CHANGE_DELAY: Duration = Duration::from_millis(500);
/// 注册失败或监听线程退出后的重试间隔。
pub(super) const RETRY_DELAY: Duration = Duration::from_secs(5);

/// 监听器只需要能回答「线程还活着吗」，快捷键和按键映射钩子都接得进来。
pub(super) trait Listener {
    fn is_alive(&self) -> bool;
}

impl Listener for GlobalHotkey {
    fn is_alive(&self) -> bool {
        GlobalHotkey::is_alive(self)
    }
}

impl Listener for KeymapListener {
    fn is_alive(&self) -> bool {
        KeymapListener::is_alive(self)
    }
}

/// 一次 [`sync`] 的结果；调用方据此决定记什么日志。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Synced {
    /// 状态没变、限流中，或者本来就不需要注册。
    Idle,
    /// 监听线程意外退出，已重新注册。
    Restarted,
    /// 已注销（想要的状态是「不注册」）。
    Cleared,
    /// 新注册成功。
    Registered,
    /// 注册失败；`first` 表示这是不是第一次报同一个错误（去重后只记一次日志）。
    Failed { first: bool, error: String },
}

/// 清空一个槽位：释放监听器并忘掉上次尝试（录制快捷键、关掉功能时用）。
pub(super) fn reset<L>(
    listener: &mut Option<L>,
    attempted: &mut Option<String>,
    error: &mut Option<String>,
    tried_at: &mut Option<Instant>,
) {
    *listener = None;
    *attempted = None;
    *error = None;
    *tried_at = None;
}

/// 按需注册：状态没变就只探活，变了才重建；失败后按 [`RETRY_DELAY`] 重试。
///
/// `desired` 是「想要注册的标识」（快捷键的显示文本 / 映射规则签名），
/// 用它可以判断要不要重建；`spawn` 只在真的要注册时调用。
pub(super) fn sync<L, F>(
    listener: &mut Option<L>,
    attempted: &mut Option<String>,
    error: &mut Option<String>,
    tried_at: &mut Option<Instant>,
    desired: Option<String>,
    spawn: F,
) -> Synced
where
    L: Listener,
    F: FnOnce() -> Result<L, String>,
{
    let changed = *attempted != desired;
    let mut restarted = false;
    if !changed {
        // 监听线程意外退出时系统已经把热键注销了，但界面还写着「已注册」。
        // 这里主动发现并重新注册，否则就会一直「提示已注册、按键没反应」。
        if listener.as_ref().is_some_and(Listener::is_alive) {
            return Synced::Idle;
        }
        if listener.is_some() {
            *listener = None;
            *error = None;
            restarted = true;
        } else if desired.is_none() {
            return Synced::Idle;
        }
    }
    let delay = if changed { CHANGE_DELAY } else { RETRY_DELAY };
    if tried_at.is_some_and(|at| at.elapsed() < delay) {
        return Synced::Idle;
    }
    *attempted = desired.clone();
    *tried_at = Some(Instant::now());
    // 先释放旧的，避免同一个键重复注册。
    *listener = None;
    let Some(_) = desired else {
        *error = None;
        return Synced::Cleared;
    };
    match spawn() {
        Ok(new_listener) => {
            *listener = Some(new_listener);
            *error = None;
            if restarted {
                Synced::Restarted
            } else {
                Synced::Registered
            }
        }
        Err(err) => {
            let first = error.as_deref() != Some(err.as_str());
            *error = Some(err.clone());
            Synced::Failed { first, error: err }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    /// 假监听器：`alive` 由测试控制，`spawn` 次数记在 `spawns` 里。
    struct Fake {
        alive: Arc<AtomicBool>,
    }

    impl Listener for Fake {
        fn is_alive(&self) -> bool {
            self.alive.load(Ordering::Relaxed)
        }
    }

    struct Slot {
        listener: Option<Fake>,
        attempted: Option<String>,
        error: Option<String>,
        tried_at: Option<Instant>,
    }

    impl Slot {
        fn new() -> Self {
            Self {
                listener: None,
                attempted: None,
                error: None,
                tried_at: None,
            }
        }

        fn sync(&mut self, desired: Option<&str>, alive: bool, spawns: &AtomicUsize) -> Synced {
            sync(
                &mut self.listener,
                &mut self.attempted,
                &mut self.error,
                &mut self.tried_at,
                desired.map(str::to_string),
                || {
                    spawns.fetch_add(1, Ordering::Relaxed);
                    Ok(Fake {
                        alive: Arc::new(AtomicBool::new(alive)),
                    })
                },
            )
        }

        /// 让限流窗口过去，模拟「5 秒 / 500 毫秒后重试」。
        fn wait_out_delay(&mut self) {
            self.tried_at = Some(Instant::now() - RETRY_DELAY);
        }
    }

    #[test]
    fn registers_once_and_stays_idle_while_alive() {
        let mut slot = Slot::new();
        let spawns = AtomicUsize::new(0);
        assert_eq!(slot.sync(Some("F9"), true, &spawns), Synced::Registered);
        assert_eq!(slot.sync(Some("F9"), true, &spawns), Synced::Idle);
        assert_eq!(spawns.load(Ordering::Relaxed), 1, "状态没变就不该重建");
    }

    #[test]
    fn change_is_throttled_but_retry_after_death_is_immediate_after_delay() {
        let mut slot = Slot::new();
        let spawns = AtomicUsize::new(0);
        slot.sync(Some("F9"), true, &spawns);
        // 改快捷键后立刻再改：500ms 内不重复注册。
        assert_eq!(slot.sync(Some("F10"), true, &spawns), Synced::Idle);
        assert_eq!(spawns.load(Ordering::Relaxed), 1);
        slot.wait_out_delay();
        assert_eq!(slot.sync(Some("F10"), true, &spawns), Synced::Registered);
        assert_eq!(spawns.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn dead_listener_is_rebuilt_and_reported() {
        let mut slot = Slot::new();
        let spawns = AtomicUsize::new(0);
        slot.sync(Some("F9"), true, &spawns);
        // 线程死了：把 listener 换成一个已退出的，模拟意外退出。
        slot.listener = Some(Fake {
            alive: Arc::new(AtomicBool::new(false)),
        });
        slot.wait_out_delay();
        assert_eq!(slot.sync(Some("F9"), true, &spawns), Synced::Restarted);
        assert_eq!(spawns.load(Ordering::Relaxed), 2);
        // 退出后重建失败：第一次要报，重复的同一个错误不报。
        slot.listener = Some(Fake {
            alive: Arc::new(AtomicBool::new(false)),
        });
        slot.wait_out_delay();
        let failed = super::sync(
            &mut slot.listener,
            &mut slot.attempted,
            &mut slot.error,
            &mut slot.tried_at,
            Some("F9".to_string()),
            || Err("按键被占用".to_string()),
        );
        assert_eq!(
            failed,
            Synced::Failed {
                first: true,
                error: "按键被占用".to_string()
            }
        );
        slot.wait_out_delay();
        let again = super::sync(
            &mut slot.listener,
            &mut slot.attempted,
            &mut slot.error,
            &mut slot.tried_at,
            Some("F9".to_string()),
            || Err("按键被占用".to_string()),
        );
        assert_eq!(
            again,
            Synced::Failed {
                first: false,
                error: "按键被占用".to_string()
            },
            "同一个错误只记一次日志"
        );
    }

    #[test]
    fn empty_desired_unregisters_once() {
        let mut slot = Slot::new();
        let spawns = AtomicUsize::new(0);
        slot.sync(Some("F9"), true, &spawns);
        // 注销也走变更限流（跟改快捷键一样，避免输入过程中反复装卸）。
        slot.wait_out_delay();
        assert_eq!(slot.sync(None, true, &spawns), Synced::Cleared);
        assert!(slot.listener.is_none());
        assert!(slot.attempted.is_none());
        assert_eq!(slot.sync(None, true, &spawns), Synced::Idle);
        assert_eq!(spawns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn reset_forgets_everything() {
        let mut slot = Slot::new();
        let spawns = AtomicUsize::new(0);
        slot.sync(Some("F9"), true, &spawns);
        super::reset(
            &mut slot.listener,
            &mut slot.attempted,
            &mut slot.error,
            &mut slot.tried_at,
        );
        assert!(slot.listener.is_none());
        assert!(slot.attempted.is_none());
        assert!(slot.error.is_none());
        assert!(slot.tried_at.is_none());
    }
}
