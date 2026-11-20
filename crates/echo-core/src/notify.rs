//! Notification rate limiting: at most one notification per repo per window;
//! anything arriving inside the window is merged into one follow-up.

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const WINDOW: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Default)]
struct Slot {
    last_sent: Option<Instant>,
    pending: Vec<Note>,
    flush_scheduled: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Offer {
    /// Send this now.
    Send(Note),
    /// Queued; schedule a flush after this delay.
    ScheduleFlush(Duration),
    /// Queued; a flush is already scheduled.
    Queued,
}

#[derive(Debug)]
pub struct RateLimiter {
    window: Duration,
    slots: HashMap<String, Slot>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(WINDOW)
    }
}

impl RateLimiter {
    pub fn new(window: Duration) -> Self {
        Self { window, slots: HashMap::new() }
    }

    pub fn offer(&mut self, key: &str, note: Note, now: Instant) -> Offer {
        let slot = self.slots.entry(key.to_string()).or_default();
        if slot.flush_scheduled {
            // Keep order: anything after a queued note waits for the flush.
            slot.pending.push(note);
            return Offer::Queued;
        }
        match slot.last_sent {
            Some(t) if now.saturating_duration_since(t) < self.window => {
                slot.pending.push(note);
                if slot.flush_scheduled {
                    Offer::Queued
                } else {
                    slot.flush_scheduled = true;
                    Offer::ScheduleFlush(self.window - now.saturating_duration_since(t))
                }
            }
            _ => {
                slot.last_sent = Some(now);
                Offer::Send(note)
            }
        }
    }

    /// Called when a scheduled flush fires: returns the merged pending note.
    pub fn flush(&mut self, key: &str, now: Instant) -> Option<Note> {
        let slot = self.slots.get_mut(key)?;
        slot.flush_scheduled = false;
        if slot.pending.is_empty() {
            return None;
        }
        slot.last_sent = Some(now);
        Some(merge(std::mem::take(&mut slot.pending)))
    }
}

fn merge(mut notes: Vec<Note>) -> Note {
    if notes.len() == 1 {
        return notes.pop().unwrap();
    }
    let title = notes.last().map(|n| n.title.clone()).unwrap_or_default();
    let n = notes.len();
    let mut lines: Vec<String> =
        notes.iter().rev().take(3).map(|n| n.body.lines().next().unwrap_or("").to_string()).collect();
    lines.reverse();
    if n > 3 {
        lines.insert(0, format!("{} more updates…", n - 3));
    }
    Note { title, body: lines.join("\n") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(b: &str) -> Note {
        Note { title: "Echo · repo".into(), body: b.into() }
    }

    #[test]
    fn rate_limits_and_merges() {
        let t0 = Instant::now();
        let mut rl = RateLimiter::default();
        assert_eq!(rl.offer("r", note("a"), t0), Offer::Send(note("a")));
        assert_eq!(rl.offer("r", note("b"), t0 + Duration::from_secs(2)), Offer::ScheduleFlush(Duration::from_secs(8)));
        assert_eq!(rl.offer("r", note("c"), t0 + Duration::from_secs(3)), Offer::Queued);
        // Another repo is independent.
        assert_eq!(rl.offer("other", note("x"), t0 + Duration::from_secs(3)), Offer::Send(note("x")));
        let merged = rl.flush("r", t0 + Duration::from_secs(10)).unwrap();
        assert_eq!(merged.body, "b\nc");
        assert_eq!(rl.flush("r", t0 + Duration::from_secs(10)), None);
        // Window restarts from the flush.
        assert!(matches!(rl.offer("r", note("d"), t0 + Duration::from_secs(15)), Offer::ScheduleFlush(_)));
        assert!(matches!(rl.offer("r", note("e"), t0 + Duration::from_secs(25)), Offer::Queued));
    }

    #[test]
    fn merge_caps_lines() {
        let m = merge((0..5).map(|i| note(&format!("n{i}"))).collect());
        assert_eq!(m.body, "2 more updates…\nn2\nn3\nn4");
    }
}
