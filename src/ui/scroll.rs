//! Which scroller a wheel or trackpad gesture belongs to.
//!
//! egui hands every scroll event to whatever scroll area is under the pointer that
//! frame, with both axes at once. Two things follow that macOS doesn't do:
//! * A trackpad swipe down the page is never perfectly vertical, so the small sideways
//!   part slid the shelves under the pointer left and right while the page scrolled.
//! * A gesture's tail (trackpad momentum, the wheel's glide) went to whatever the
//!   pointer drifted onto, so the page's scroll carried on in the sidebar.
//!
//! Here a gesture is locked to its main axis, and to the pane (sidebar, drawer or the
//! pages) it started over: events after the pointer leaves that pane are dropped.

use eframe::egui;

/// A pause this long between scroll events ends the gesture.
const GAP: f64 = 0.3;
/// Trackpad travel, in points, before the gesture's axis is decided.
const AXIS_DECIDE: f32 = 4.0;

#[derive(Clone, Debug, Default)]
pub struct Gesture {
    /// Time of the last scroll event, if a gesture is under way.
    last: Option<f64>,
    /// Was it a mouse wheel (rather than a trackpad and its momentum)?
    wheel: bool,
    /// The pane it started over: an index into the panes, `None` for the pages.
    pane: Option<usize>,
    /// The axis it's locked to: 0 = x, 1 = y.
    axis: Option<usize>,
    /// Travel so far, while the axis is undecided.
    travel: egui::Vec2,
    /// Where the pointer is, as of the events seen so far.
    pointer: Option<egui::Pos2>,
}

impl Gesture {
    /// Filter this frame's scroll events: lock each gesture to one axis and one pane.
    /// `panes` are the side panes drawn last frame; everything else is the pages.
    /// Returns `false` if the pointer has left the gesture's pane, so a glide still
    /// pending should stop.
    pub fn route(&mut self, events: &mut Vec<egui::Event>, time: f64, panes: &[egui::Rect]) -> bool {
        let pane_at = |p: Option<egui::Pos2>| p.and_then(|p| panes.iter().position(|r| r.contains(p)));
        events.retain_mut(|e| match e {
            egui::Event::PointerMoved(p) => {
                self.pointer = Some(*p);
                true
            }
            egui::Event::PointerGone => {
                self.pointer = None;
                true
            }
            // Ctrl/Cmd + wheel zooms; it's no scroll.
            egui::Event::MouseWheel { modifiers, .. } if modifiers.command || modifiers.ctrl => true,
            egui::Event::MouseWheel { delta, phase, source, .. } => {
                let wheel = matches!(source, egui::MouseWheelSource::Wheel);
                let starts = *phase == egui::TouchPhase::Start && *source != egui::MouseWheelSource::Momentum;
                if starts || wheel != self.wheel || self.last.is_none_or(|t| time - t > GAP) {
                    self.wheel = wheel;
                    self.pane = pane_at(self.pointer);
                    self.axis = None;
                    self.travel = egui::Vec2::ZERO;
                }
                self.last = Some(time);
                if pane_at(self.pointer) != self.pane {
                    return false;
                }
                let axis = match self.axis {
                    Some(a) => a,
                    None => {
                        self.travel += delta.abs();
                        let a = usize::from(self.travel.y >= self.travel.x);
                        if self.travel.max_elem() >= AXIS_DECIDE {
                            self.axis = Some(a);
                        }
                        a
                    }
                };
                delta[1 - axis] = 0.0;
                true
            }
            _ => true,
        });
        self.last.is_none_or(|t| time - t > GAP) || pane_at(self.pointer) == self.pane
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wheel(dx: f32, dy: f32, phase: egui::TouchPhase, source: egui::MouseWheelSource) -> egui::Event {
        egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(dx, dy), phase, modifiers: egui::Modifiers::NONE, source }
    }
    fn pad(dx: f32, dy: f32) -> egui::Event {
        wheel(dx, dy, egui::TouchPhase::Move, egui::MouseWheelSource::Trackpad)
    }
    fn deltas(events: &[egui::Event]) -> Vec<egui::Vec2> {
        events.iter().filter_map(|e| if let egui::Event::MouseWheel { delta, .. } = e { Some(*delta) } else { None }).collect()
    }
    fn sidebar() -> Vec<egui::Rect> {
        vec![egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(200.0, 800.0))]
    }

    #[test]
    fn a_mostly_vertical_swipe_scrolls_only_vertically() {
        let mut g = Gesture::default();
        let mut ev = vec![egui::Event::PointerMoved(egui::pos2(500.0, 300.0)), wheel(1.0, -3.0, egui::TouchPhase::Start, egui::MouseWheelSource::Trackpad), pad(2.0, -6.0)];
        g.route(&mut ev, 0.0, &sidebar());
        let mut more = vec![pad(3.0, -1.0)];
        g.route(&mut more, 0.016, &sidebar());
        assert_eq!(deltas(&ev), vec![egui::vec2(0.0, -3.0), egui::vec2(0.0, -6.0)]);
        assert_eq!(deltas(&more), vec![egui::vec2(0.0, -1.0)]);
    }

    #[test]
    fn a_sideways_swipe_scrolls_only_sideways() {
        let mut g = Gesture::default();
        let mut ev = vec![egui::Event::PointerMoved(egui::pos2(500.0, 300.0)), pad(-8.0, 1.0), pad(-5.0, 2.0)];
        g.route(&mut ev, 0.0, &sidebar());
        assert_eq!(deltas(&ev), vec![egui::vec2(-8.0, 0.0), egui::vec2(-5.0, 0.0)]);
    }

    #[test]
    fn a_page_scroll_doesnt_carry_into_the_sidebar() {
        let mut g = Gesture::default();
        let mut ev = vec![egui::Event::PointerMoved(egui::pos2(500.0, 300.0)), pad(0.0, -10.0)];
        assert!(g.route(&mut ev, 0.0, &sidebar()));
        assert_eq!(deltas(&ev).len(), 1);
        // The momentum goes on after the pointer moved over the sidebar.
        let mut tail = vec![egui::Event::PointerMoved(egui::pos2(100.0, 300.0)), wheel(0.0, -8.0, egui::TouchPhase::Start, egui::MouseWheelSource::Momentum)];
        assert!(!g.route(&mut tail, 0.05, &sidebar()));
        assert!(deltas(&tail).is_empty());
        // A new swipe there scrolls the sidebar.
        let mut next = vec![wheel(0.0, -4.0, egui::TouchPhase::Start, egui::MouseWheelSource::Trackpad)];
        assert!(g.route(&mut next, 0.1, &sidebar()));
        assert_eq!(deltas(&next), vec![egui::vec2(0.0, -4.0)]);
    }

    #[test]
    fn a_wheel_spun_on_after_a_pause_scrolls_where_the_pointer_is() {
        let mut g = Gesture::default();
        let notch = || wheel(0.0, -1.0, egui::TouchPhase::Move, egui::MouseWheelSource::Wheel);
        let mut ev = vec![egui::Event::PointerMoved(egui::pos2(500.0, 300.0)), notch()];
        g.route(&mut ev, 0.0, &sidebar());
        let mut moved = vec![egui::Event::PointerMoved(egui::pos2(100.0, 300.0)), notch()];
        g.route(&mut moved, 0.1, &sidebar());
        assert!(deltas(&moved).is_empty());
        let mut later = vec![notch()];
        g.route(&mut later, 0.6, &sidebar());
        assert_eq!(deltas(&later).len(), 1);
    }
}
