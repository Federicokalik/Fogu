//! Cursor blink state: a timer runs only while the window is focused, blinking is enabled and the
//! user was active in the last [`IDLE_AFTER`]; after that the cursor stays solid so an idle window
//! costs nothing.

use std::time::{Duration, Instant};

/// Half-period of the blink.
pub const PERIOD: Duration = Duration::from_millis(530);
/// Blinking stops (cursor solid) after this long without user activity.
pub const IDLE_AFTER: Duration = Duration::from_secs(10);

/// Phase + bookkeeping of the blink timer.
#[derive(Debug)]
pub struct Blink {
    /// Whether the cursor is drawn in the current phase.
    pub on: bool,
    last_activity: Instant,
    /// A timer message is in flight; at most one ever is.
    armed: bool,
}

impl Blink {
    /// Solid cursor, activity "now".
    pub fn new(now: Instant) -> Self {
        Self {
            on: true,
            last_activity: now,
            armed: false,
        }
    }

    /// User input or focus: show the cursor solid again and restart the idle clock.
    pub fn activity(&mut self, now: Instant) {
        self.on = true;
        self.last_activity = now;
    }

    fn should_run(&self, now: Instant, enabled: bool, focused: bool) -> bool {
        enabled && focused && now.saturating_duration_since(self.last_activity) < IDLE_AFTER
    }

    /// Whether a new timer must be started now (none in flight and blinking is due). Marks it
    /// in flight when `true`.
    pub fn arm(&mut self, now: Instant, enabled: bool, focused: bool) -> bool {
        if self.armed || !self.should_run(now, enabled, focused) {
            return false;
        }
        self.armed = true;
        true
    }

    /// The timer fired: toggles the phase and returns `true` when another tick is wanted; once
    /// blinking must stop the cursor is left solid.
    pub fn tick(&mut self, now: Instant, enabled: bool, focused: bool) -> bool {
        self.armed = false;
        if !self.should_run(now, enabled, focused) {
            self.on = true;
            return false;
        }
        self.on = !self.on;
        self.armed = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blinks_then_goes_solid_after_ten_idle_seconds() {
        let t0 = Instant::now();
        let mut blink = Blink::new(t0);
        assert!(blink.arm(t0, true, true));
        assert!(!blink.arm(t0, true, true), "one timer at a time");
        assert!(blink.tick(t0 + PERIOD, true, true));
        assert!(!blink.on, "first tick hides it");
        assert!(blink.tick(t0 + PERIOD * 2, true, true));
        assert!(blink.on);
        assert!(blink.tick(t0 + Duration::from_millis(9_999), true, true));
        assert!(
            !blink.tick(t0 + IDLE_AFTER, true, true),
            "idle: timer stops"
        );
        assert!(blink.on, "and the cursor is left visible");
        assert!(
            !blink.arm(t0 + IDLE_AFTER, true, true),
            "no restart while idle"
        );
    }

    #[test]
    fn activity_restarts_the_clock_and_shows_the_cursor() {
        let t0 = Instant::now();
        let mut blink = Blink::new(t0);
        blink.tick(t0 + PERIOD, true, true);
        assert!(!blink.on);
        let later = t0 + IDLE_AFTER + Duration::from_secs(5);
        assert!(
            !blink.tick(later, true, true),
            "idle by now: the timer stops"
        );
        blink.activity(later);
        assert!(blink.on);
        assert!(blink.arm(later, true, true));
    }

    #[test]
    fn unfocused_or_disabled_never_blinks() {
        let t0 = Instant::now();
        for (enabled, focused) in [(false, true), (true, false), (false, false)] {
            let mut blink = Blink::new(t0);
            assert!(!blink.arm(t0, enabled, focused));
            assert!(!blink.tick(t0, enabled, focused));
            assert!(blink.on);
        }
    }
}
