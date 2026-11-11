//! Window helpers that winit handles on macOS: full screen, the work area, monitor
//! checks and rounded corners. Kept as no-ops so callers don't need to change.

/// Enter or leave full screen. Always false: winit does it on macOS.
pub fn set(_on: bool) -> bool {
    false
}

/// The work area of the monitor the window is on. Not available on macOS.
pub fn work_area() -> Option<egui::Rect> {
    None
}

/// Whether some of `rect` (physical pixels) is on a connected monitor. True, since
/// the app can't ask on macOS.
pub fn on_a_monitor(_rect: egui::Rect) -> bool {
    true
}

/// Round the window's corners. A no-op on macOS.
pub fn set_rounded(_radius: Option<f32>) {}
