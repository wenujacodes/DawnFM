//! The sleep timer: pause after a while, or once this song ends. Not kept across launches.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::{App, Repeat};

/// The lengths now playing's menu offers, in minutes.
pub const MINUTES: [u64; 5] = [15, 30, 45, 60, 90];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sleep {
    #[default]
    Off,
    /// Pause once this instant passes.
    At(Instant),
    /// Pause when the playing song finishes, instead of going on to the next.
    EndOfSong,
}

impl Sleep {
    pub fn after(minutes: u64, now: Instant) -> Self {
        Sleep::At(now + Duration::from_secs(minutes * 60))
    }

    pub fn active(&self) -> bool {
        *self != Sleep::Off
    }

    /// Time left on a timed one.
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        match self {
            Sleep::At(at) => Some(at.saturating_duration_since(now)),
            _ => None,
        }
    }

    /// A timed one that has run out.
    pub fn due(&self, now: Instant) -> bool {
        matches!(self, Sleep::At(at) if now >= *at)
    }

    /// What the menu row shows on its right.
    pub fn label(&self, now: Instant) -> String {
        match self {
            Sleep::Off => "Off".into(),
            Sleep::At(_) => fmt_remaining(self.remaining(now).unwrap_or_default()),
            Sleep::EndOfSong => "End of song".into(),
        }
    }
}

/// 1:30:00, 14:59, 0:05: whole seconds, rounded up so it reads 0:00 only as it fires.
pub fn fmt_remaining(d: Duration) -> String {
    let s = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

impl App {
    /// Each frame: pause once a timed one runs out, and wake up for it.
    pub(super) fn tick_sleep(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self.sleep.due(now) {
            self.sleep_fire();
        } else if let Some(left) = self.sleep.remaining(now) {
            ctx.request_repaint_after(left);
        }
    }

    /// Pause (not quit) and turn the timer off.
    pub(super) fn sleep_fire(&mut self) {
        self.sleep = Sleep::Off;
        if self.playing() {
            self.toggle();
        }
        self.toast("Sleep timer: playback paused");
    }

    /// The song ran out with "End of this song" set: the next one is cued, not played,
    /// and starts from the top on Play (as last session's song does).
    pub(super) fn sleep_at_song_end(&mut self) {
        self.sleep = Sleep::Off;
        if self.queue.repeat != Repeat::One {
            self.queue.advance();
        }
        if let Some(track) = self.current().cloned() {
            self.mix_from = None;
            self.playing_as = None;
            self.audio.status.duration_ms.store(track.duration.map_or(0, |d| d as u64 * 1000), Ordering::Relaxed);
            self.audio.status.position_ms.store(0, Ordering::Relaxed);
            self.resume_at = Some(Duration::ZERO);
            self.resume_counts = true;
            self.request_art();
            self.show_in_media(&track);
            self.refresh_lyrics(track);
        }
        self.toast("Sleep timer: paused at the end of the song");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_whole_seconds_rounded_up() {
        assert_eq!(fmt_remaining(Duration::ZERO), "0:00");
        assert_eq!(fmt_remaining(Duration::from_millis(4_200)), "0:05");
        assert_eq!(fmt_remaining(Duration::from_secs(14 * 60 + 59)), "14:59");
        assert_eq!(fmt_remaining(Duration::from_secs(60 * 60)), "1:00:00");
        assert_eq!(fmt_remaining(Duration::from_secs(90 * 60 - 1)), "1:29:59");
    }

    #[test]
    fn counts_down_to_the_deadline() {
        let t0 = Instant::now();
        let s = Sleep::after(15, t0);
        assert!(s.active());
        assert_eq!(s.remaining(t0 + Duration::from_secs(60)), Some(Duration::from_secs(14 * 60)));
        assert_eq!(s.label(t0 + Duration::from_secs(60)), "14:00");
        assert!(!s.due(t0 + Duration::from_secs(15 * 60 - 1)));
        assert!(s.due(t0 + Duration::from_secs(15 * 60)));
        assert_eq!(s.remaining(t0 + Duration::from_secs(16 * 60)), Some(Duration::ZERO));
    }

    #[test]
    fn end_of_song_and_off_never_come_due() {
        let later = Instant::now() + Duration::from_secs(24 * 3600);
        for s in [Sleep::Off, Sleep::EndOfSong] {
            assert!(!s.due(later));
            assert_eq!(s.remaining(later), None);
        }
        assert!(!Sleep::Off.active());
        assert!(Sleep::EndOfSong.active());
        assert_eq!(Sleep::EndOfSong.label(later), "End of song");
    }
}
