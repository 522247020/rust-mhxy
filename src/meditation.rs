//! 打坐倒计时：状态机与显示文本。移植自 `mhxy-dz` 的 C# 版本。
//!
//! 用单调时钟（`Instant`）计时，暂停时把剩余时间冻结；
//! 结束后停在 00:00 / 0，不会继续倒数。

use std::time::{Duration, Instant};

pub const DEFAULT_DURATION_SECONDS: u32 = 180;
pub const MIN_DURATION_SECONDS: u32 = 60;
pub const MAX_DURATION_SECONDS: u32 = 180 * 60;
/// 到点后数字闪烁的时长。
pub const FINISH_FLASH: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Running,
    Paused,
    Finished,
}

#[derive(Debug)]
pub struct MeditationTimer {
    duration: Duration,
    remaining: Duration,
    ends_at: Option<Instant>,
    state: State,
    finished_at: Option<Instant>,
}

impl MeditationTimer {
    pub fn new(duration_seconds: u32) -> Self {
        let duration = Duration::from_secs(clamp_duration(duration_seconds) as u64);
        Self {
            duration,
            remaining: duration,
            ends_at: None,
            state: State::Idle,
            finished_at: None,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// 修改时长会重置计时，和 C# 版 `SetDuration` 一致。
    pub fn set_duration_seconds(&mut self, seconds: u32) {
        self.duration = Duration::from_secs(clamp_duration(seconds) as u64);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.remaining = self.duration;
        self.ends_at = None;
        self.finished_at = None;
    }

    /// 开始 / 重新计时。
    pub fn restart(&mut self) {
        self.remaining = self.duration;
        self.ends_at = Some(Instant::now() + self.remaining);
        self.state = State::Running;
        self.finished_at = None;
    }

    pub fn pause(&mut self) {
        if self.state != State::Running {
            return;
        }
        self.tick();
        if self.state == State::Running {
            self.remaining = self.remaining_now();
            self.ends_at = None;
            self.state = State::Paused;
        }
    }

    pub fn resume(&mut self) {
        if self.state != State::Paused {
            return;
        }
        self.ends_at = Some(Instant::now() + self.remaining);
        self.state = State::Running;
    }

    /// 悬浮窗单击：计时中暂停，暂停时继续，空闲 / 完成时重新开始。
    pub fn toggle(&mut self) {
        match self.state {
            State::Running => self.pause(),
            State::Paused => self.resume(),
            State::Idle | State::Finished => self.restart(),
        }
    }

    /// 刷新剩余时间，归零时切到完成状态。
    pub fn tick(&mut self) {
        if self.state != State::Running {
            return;
        }
        let remaining = self.remaining_now();
        self.remaining = remaining;
        if remaining.is_zero() {
            self.ends_at = None;
            self.state = State::Finished;
            self.finished_at = Some(Instant::now());
        }
    }

    pub fn remaining(&self) -> Duration {
        if self.state == State::Running {
            self.remaining_now()
        } else {
            self.remaining
        }
    }

    /// 悬浮窗文字：`seconds` 只显示秒数，`clock` 显示分:秒。
    pub fn text(&self, clock: bool) -> String {
        let millis = self.remaining().as_millis() as u64;
        // 向上取整，避免还差 0.9 秒就显示 0。
        let seconds = millis.div_ceil(1000);
        if clock {
            format!("{:02}:{:02}", seconds / 60, seconds % 60)
        } else {
            seconds.to_string()
        }
    }

    /// 刚结束时返回经过的时间，用于闪烁提示。
    pub fn finished_elapsed(&self) -> Option<Duration> {
        self.finished_at.map(|at| at.elapsed())
    }

    fn remaining_now(&self) -> Duration {
        self.ends_at
            .map(|end| end.saturating_duration_since(Instant::now()))
            .unwrap_or_default()
    }
}

pub fn clamp_duration(seconds: u32) -> u32 {
    seconds.clamp(MIN_DURATION_SECONDS, MAX_DURATION_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn counts_down_and_finishes() {
        let mut timer = MeditationTimer::new(MIN_DURATION_SECONDS);
        timer.set_duration_seconds(1);
        // 低于下限会被夹到 1 分钟，这里直接验证夹取后的文字。
        assert_eq!(timer.state(), State::Idle);
        assert_eq!(timer.text(false), "60");

        timer.restart();
        assert_eq!(timer.state(), State::Running);
        sleep(Duration::from_millis(1200));
        timer.tick();
        assert_eq!(timer.state(), State::Running);
        assert!(timer.remaining() < Duration::from_secs(59));
    }

    #[test]
    fn pause_freezes_and_resume_continues() {
        let mut timer = MeditationTimer::new(120);
        timer.restart();
        sleep(Duration::from_millis(50));
        timer.pause();
        let frozen = timer.remaining();
        assert_eq!(timer.state(), State::Paused);
        sleep(Duration::from_millis(80));
        timer.tick();
        assert_eq!(timer.remaining(), frozen, "暂停后剩余时间不应变化");
        timer.resume();
        assert_eq!(timer.state(), State::Running);
    }

    #[test]
    fn toggle_walks_through_states() {
        let mut timer = MeditationTimer::new(120);
        timer.toggle();
        assert_eq!(timer.state(), State::Running);
        timer.toggle();
        assert_eq!(timer.state(), State::Paused);
        timer.toggle();
        assert_eq!(timer.state(), State::Running);
        timer.reset();
        assert_eq!(timer.state(), State::Idle);
    }

    #[test]
    fn formats_seconds_and_clock() {
        let mut timer = MeditationTimer::new(180);
        assert_eq!(timer.text(false), "180");
        assert_eq!(timer.text(true), "03:00");
        timer.set_duration_seconds(65);
        assert_eq!(timer.text(false), "65");
        assert_eq!(timer.text(true), "01:05");
    }

    #[test]
    fn finishes_at_zero_and_stays() {
        let mut timer = MeditationTimer::new(MIN_DURATION_SECONDS);
        timer.restart();
        // 直接把截止时间倒回过去，模拟到点。
        timer.ends_at = Some(Instant::now() - Duration::from_millis(1));
        timer.tick();
        assert_eq!(timer.state(), State::Finished);
        assert_eq!(timer.text(false), "0");
        assert_eq!(timer.text(true), "00:00");
        timer.tick();
        assert_eq!(timer.state(), State::Finished);
        assert!(timer.finished_elapsed().is_some());
    }

    #[test]
    fn clamps_out_of_range_durations() {
        assert_eq!(clamp_duration(0), MIN_DURATION_SECONDS);
        assert_eq!(clamp_duration(90), 90);
        assert_eq!(clamp_duration(999_999), MAX_DURATION_SECONDS);
    }
}
