//! Visual design tokens: fonts, icons, colours, motion.

use crate::sync::{MutexExt, RwLockExt};
use std::sync::Arc;

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId};

// ------------------------------------------------------------------ colours

/// Base surface under the animated background (seen while it fades in).
const BG: Color32 = Color32::from_rgb(0x11, 0x11, 0x11);
/// Sidebar surface colour in dark mode: #222629 (RGB: 34, 38, 41).
pub const SIDEBAR_BG_DARK: Color32 = Color32::from_rgb(0x22, 0x26, 0x29);
const TEXT: Color32 = Color32::from_rgb(242, 240, 245);
const TEXT_DIM: Color32 = Color32::from_rgb(178, 172, 186);
const TEXT_FAINT: Color32 = Color32::from_rgb(130, 124, 140);
/// Light mode's counterparts.
const BG_LIGHT: Color32 = Color32::from_rgb(246, 244, 248);
const TEXT_LIGHT: Color32 = Color32::from_rgb(22, 20, 26);
const TEXT_DIM_LIGHT: Color32 = Color32::from_rgb(86, 82, 94);
const TEXT_FAINT_LIGHT: Color32 = Color32::from_rgb(124, 118, 132);
/// Built-in red and blue tokens (Blue is the default accent; Red is used for errors and the Red swatch).
pub const RED: Color32 = Color32::from_rgb(0xF2, 0x36, 0x4D);
pub const BLUE: Color32 = Color32::from_rgb(0x3D, 0x8B, 0xFF);

/// Light or dark, as the app picked it for what's being drawn (see `set_mode`): the
/// colours below follow it. Now playing always draws dark, over the album art.
static LIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn light() -> bool {
    LIGHT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Draw what follows light or dark: the colours here and egui's style (menus, fields,
/// text) change together. Returns the mode before, to go back to.
pub fn set_mode(ctx: &egui::Context, light: bool) -> bool {
    let was = LIGHT.swap(light, std::sync::atomic::Ordering::Relaxed);
    if was != light {
        ctx.global_style_mut(|s| s.visuals = visuals());
    }
    was
}

/// Main text.
pub fn text() -> Color32 {
    if light() { TEXT_LIGHT } else { TEXT }
}

/// Sidebar item labels: pure black on light pages, pure white on dark ones.
pub fn label() -> Color32 {
    if light() { Color32::BLACK } else { Color32::WHITE }
}

/// Text and icons on a fill of the main text colour (a primary button, a selected chip).
pub fn on_text() -> Color32 {
    if light() { Color32::WHITE } else { Color32::from_gray(15) }
}

/// Least important text: counts, hints, placeholders.
pub fn text_faint() -> Color32 {
    if light() { TEXT_FAINT_LIGHT } else { TEXT_FAINT }
}

/// The window's base colour, under the background.
pub fn bg() -> Color32 {
    if light() { BG_LIGHT } else { BG }
}

/// Menus and popups: frosted glass blur style with translucent tint.
pub fn menu_fill() -> Color32 {
    if light() {
        Color32::from_rgba_premultiplied(246, 246, 248, 225)
    } else {
        Color32::from_rgba_premultiplied(0x11, 0x11, 0x11, 220)
    }
}

/// Dialogs and cards that float over the page.
pub fn surface_fill() -> Color32 {
    if light() { Color32::from_rgb(252, 251, 253) } else { Color32::from_rgb(0x11, 0x11, 0x11) }
}

/// Theme preset multiplier for every glass surface (f32 bits; 1.0 by default).
static GLASS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

/// White at `a` opacity, for glassy surfaces over the background (black in light mode).
pub fn glass(a: f32) -> Color32 {
    let k = f32::from_bits(GLASS.load(std::sync::atomic::Ordering::Relaxed));
    let a = ((a * k).min(1.0) * 255.0) as u8;
    if light() { Color32::from_black_alpha(a) } else { Color32::from_white_alpha(a) }
}

/// A panel over the background (sidebar, player bar, queue): darker than the page at
/// `a`, or in light mode lighter.
pub fn panel(a: f32) -> Color32 {
    if light() { Color32::from_white_alpha((a.clamp(0.0, 1.0) * 255.0) as u8) } else { shade(a) }
}

/// The active theme's secondary text colour, set with its chrome (read very often).
static DIM: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(u32::from_le_bytes(TEXT_DIM.to_array()));

/// Secondary text colour (themes may tint it).
pub fn text_dim() -> Color32 {
    if light() {
        return TEXT_DIM_LIGHT;
    }
    let [r, g, b, a] = DIM.load(std::sync::atomic::Ordering::Relaxed).to_le_bytes();
    Color32::from_rgba_premultiplied(r, g, b, a)
}

/// Light, dark, or whichever the system uses (Settings › Appearance › Mode).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn from_key(s: &str) -> Self {
        Self::ALL.into_iter().find(|a| a.key() == s).unwrap_or_default()
    }

    /// Whether this means light, given the system's mode as eframe reports it.
    pub fn is_light(self, ctx: &egui::Context) -> bool {
        match self {
            Self::Light => true,
            Self::Dark => false,
            Self::System => system_light().unwrap_or_else(|| ctx.system_theme() == Some(egui::Theme::Light)),
        }
    }
}

/// The system's light or dark mode as last seen: 0 not read yet, 1 light, 2 dark,
/// 3 can't be read. Kept fresh by `watch_system_appearance`.
static SYSTEM_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn encode_mode(m: Option<bool>) -> u8 {
    match m {
        Some(true) => 1,
        Some(false) => 2,
        None => 3,
    }
}

/// The system's light or dark mode, so switching it shows without a restart. `None`
/// where it can't be read.
fn system_light() -> Option<bool> {
    use std::sync::atomic::Ordering::Relaxed;
    if SYSTEM_MODE.load(Relaxed) == 0 {
        SYSTEM_MODE.store(encode_mode(read_system_light()), Relaxed);
    }
    match SYSTEM_MODE.load(Relaxed) {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

/// Watches the system's light or dark mode and accent colour, waking the UI the moment
/// either changes. An idle app draws nothing, so without this a switch would show only
/// once something else (the mouse, a track change) made it repaint. Safe to call every
/// frame; it starts once.
pub fn watch_system_appearance(ctx: &egui::Context) {
    use std::sync::atomic::Ordering::Relaxed;
    static STARTED: std::sync::Once = std::sync::Once::new();
    let watches_mode = read_system_light().is_some();
    let watches_accent = read_system_accent_watch_token().is_some();
    if STARTED.is_completed() || (!watches_mode && !watches_accent) {
        return;
    }
    STARTED.call_once(|| {
        let mut prev_accent = read_system_accent_watch_token();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("system-appearance".into()).spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let mut changed = false;
                if watches_mode {
                    let now = encode_mode(read_system_light());
                    if SYSTEM_MODE.swap(now, Relaxed) != now {
                        changed = true;
                    }
                }
                if watches_accent {
                    let now_accent = read_system_accent_watch_token();
                    if now_accent != prev_accent {
                        prev_accent = now_accent;
                        *SYSTEM_ACCENT.lock_unpoisoned() = None;
                        changed = true;
                    }
                }
                if changed {
                    ctx.request_repaint();
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("system appearance watcher: {e}");
        }
    });
}

/// Thread-safe change token for system accent updates on the background watcher thread.
#[cfg(target_os = "macos")]
fn read_system_accent_watch_token() -> Option<u32> {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::NSString;
    // Encode presence + signed integer (`AppleAccentColor` is missing for Multicolor, else -1..=6).
    let val: Option<Retained<AnyObject>> = unsafe {
        let defaults: Retained<AnyObject> = msg_send![objc2::class!(NSUserDefaults), standardUserDefaults];
        msg_send![&*defaults, objectForKey: &*NSString::from_str("AppleAccentColor")]
    };
    let n = match val {
        Some(obj) => {
            let v: i32 = unsafe { msg_send![&*obj, intValue] };
            (v + 16).clamp(1, 255) as u32
        }
        None => 0,
    };
    let style: u32 = match read_system_light() {
        Some(true) => 1,
        Some(false) => 2,
        None => 0,
    };
    Some((n << 2) | style)
}

#[cfg(windows)]
fn read_system_accent_watch_token() -> Option<u32> {
    read_system_accent().map(|c| u32::from_le_bytes(c.to_array()))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn read_system_accent_watch_token() -> Option<u32> {
    None
}

/// macOS: the system's appearance, from the user defaults ("Dark" while it's dark, with
/// Auto too; nothing while it's light). Not the window's own, which is what eframe
/// reports: `install_style` pins that to dark for egui, so it would never say light.
#[cfg(target_os = "macos")]
fn read_system_light() -> Option<bool> {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::NSString;
    // SAFETY: Foundation calls with the argument and return types they declare;
    // NSUserDefaults is safe to read from any thread.
    let style: Option<Retained<NSString>> = unsafe {
        let defaults: Retained<AnyObject> = msg_send![objc2::class!(NSUserDefaults), standardUserDefaults];
        msg_send![&*defaults, stringForKey: &*NSString::from_str("AppleInterfaceStyle")]
    };
    Some(!style.is_some_and(|s| s.to_string().eq_ignore_ascii_case("dark")))
}

#[cfg(not(target_os = "macos"))]
fn read_system_light() -> Option<bool> {
    None
}

/// Returns true if running on macOS higher than 15 (Darwin kernel >= 25, e.g. macOS 16 / 26+).
pub fn is_macos_above_15() -> bool {
    #[cfg(target_os = "macos")]
    {
        static IS_ABOVE_15: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *IS_ABOVE_15.get_or_init(|| {
            let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
            if unsafe { libc::uname(&mut uts) } == 0 {
                let rel = unsafe { std::ffi::CStr::from_ptr(uts.release.as_ptr()) }.to_str().unwrap_or_default();
                let major: u32 = rel.split('.').next().and_then(|s| s.parse().ok()).unwrap_or(0);
                major >= 25
            } else {
                false
            }
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Material constants and properties for macOS AppKit NSVisualEffectView.
#[cfg(target_os = "macos")]
pub mod macos_vibrancy {
    pub const NS_VISUAL_EFFECT_MATERIAL_UNDER_WINDOW_BACKGROUND: isize = 21;
    pub const NS_VISUAL_EFFECT_BLENDING_MODE_BEHIND_WINDOW: isize = 0;
    pub const NS_VISUAL_EFFECT_STATE_FOLLOWS_WINDOW_ACTIVE_STATE: isize = 0;
    pub const NS_VIEW_WIDTH_SIZABLE: usize = 2;
    pub const NS_VIEW_HEIGHT_SIZABLE: usize = 16;
    pub const NS_VIEW_AUTORESIZING_MASK: usize = NS_VIEW_WIDTH_SIZABLE | NS_VIEW_HEIGHT_SIZABLE;
    pub const NS_WINDOW_BELOW: isize = -1;
}

/// On macOS, attaches an AppKit `NSVisualEffectView` using semantic material
/// `NSVisualEffectMaterialUnderWindowBackground` and blending mode `BehindWindow` via `objc2`.
/// The effect view is positioned behind the content view and resizes with the window.
#[cfg(target_os = "macos")]
pub fn apply_native_vibrancy() {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::NSRect;
    use macos_vibrancy::*;

    unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![objc2::class!(NSApplication), sharedApplication];
        let Some(app) = app else {
            log::warn!("vibrancy: NSApplication sharedApplication returned nil");
            return;
        };

        let windows: Option<Retained<AnyObject>> = msg_send![&*app, windows];
        let Some(windows) = windows else {
            log::warn!("vibrancy: NSApplication windows returned nil");
            return;
        };

        let count: usize = msg_send![&*windows, count];
        if count == 0 {
            log::warn!("vibrancy: no NSWindow instances found");
            return;
        }

        let window: Option<Retained<AnyObject>> = msg_send![&*windows, objectAtIndex: 0usize];
        let Some(window) = window else {
            log::warn!("vibrancy: primary NSWindow is nil");
            return;
        };

        let content_view: Option<Retained<AnyObject>> = msg_send![&*window, contentView];
        let Some(content_view) = content_view else {
            log::warn!("vibrancy: window contentView is nil");
            return;
        };

        // Ensure native window shadow is active
        let () = msg_send![&*window, setHasShadow: true];

        let bounds: NSRect = msg_send![&*content_view, bounds];
        let effect_alloc: *mut AnyObject = msg_send![objc2::class!(NSVisualEffectView), alloc];
        if effect_alloc.is_null() {
            log::warn!("vibrancy: failed to allocate NSVisualEffectView");
            return;
        }
        let effect_view: *mut AnyObject = msg_send![effect_alloc, initWithFrame: bounds];
        if effect_view.is_null() {
            log::warn!("vibrancy: failed to initialize NSVisualEffectView");
            return;
        }

        let () = msg_send![effect_view, setMaterial: NS_VISUAL_EFFECT_MATERIAL_UNDER_WINDOW_BACKGROUND];
        let () = msg_send![effect_view, setBlendingMode: NS_VISUAL_EFFECT_BLENDING_MODE_BEHIND_WINDOW];
        let () = msg_send![effect_view, setState: NS_VISUAL_EFFECT_STATE_FOLLOWS_WINDOW_ACTIVE_STATE];
        let () = msg_send![effect_view, setAutoresizingMask: NS_VIEW_AUTORESIZING_MASK];

        let superview: Option<Retained<AnyObject>> = msg_send![&*content_view, superview];
        if let Some(superview) = superview {
            let () = msg_send![&*superview, addSubview: effect_view, positioned: NS_WINDOW_BELOW, relativeTo: &*content_view];
            log::info!("vibrancy: attached NSVisualEffectView behind contentView in superview");
        } else {
            let nil: *const AnyObject = std::ptr::null();
            let () = msg_send![&*content_view, addSubview: effect_view, positioned: NS_WINDOW_BELOW, relativeTo: nil];
            log::info!("vibrancy: attached NSVisualEffectView as backmost subview of contentView");
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn apply_native_vibrancy() {}

/// Lowers the macOS `NSTitlebarContainerView` slightly so the window's traffic light buttons
/// sit on the same ~22 pt vertical centerline as the floating header capsules.
#[cfg(target_os = "macos")]
pub fn align_macos_traffic_lights() {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool};
    use objc2_foundation::{NSPoint, NSRect};

    const EXTRA_TOP: f64 = 7.0;
    const NS_FULLSCREEN_MASK: usize = 1 << 14;

    unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![objc2::class!(NSApplication), sharedApplication];
        let Some(app) = app else { return };
        let windows: Option<Retained<AnyObject>> = msg_send![&*app, windows];
        let Some(windows) = windows else { return };
        let count: usize = msg_send![&*windows, count];
        if count == 0 {
            return;
        }
        let window: Option<Retained<AnyObject>> = msg_send![&*windows, objectAtIndex: 0usize];
        let Some(window) = window else { return };
        let mask: usize = msg_send![&*window, styleMask];
        if (mask & NS_FULLSCREEN_MASK) != 0 {
            return;
        }
        let close_btn: Option<Retained<AnyObject>> = msg_send![&*window, standardWindowButton: 0usize];
        let Some(close_btn) = close_btn else { return };
        let titlebar_view: Option<Retained<AnyObject>> = msg_send![&*close_btn, superview];
        let Some(titlebar_view) = titlebar_view else { return };
        let container: Option<Retained<AnyObject>> = msg_send![&*titlebar_view, superview];
        let Some(container) = container else { return };
        let theme_frame: Option<Retained<AnyObject>> = msg_send![&*container, superview];
        let Some(theme_frame) = theme_frame else { return };

        let theme_bounds: NSRect = msg_send![&*theme_frame, bounds];
        let container_frame: NSRect = msg_send![&*container, frame];
        let flipped: Bool = msg_send![&*theme_frame, isFlipped];
        let target_y = if flipped.as_bool() {
            EXTRA_TOP
        } else {
            (theme_bounds.size.height - container_frame.size.height - EXTRA_TOP).max(0.0)
        };
        if (container_frame.origin.y - target_y).abs() > 0.5 {
            let origin = NSPoint::new(container_frame.origin.x, target_y);
            let () = msg_send![&*container, setFrameOrigin: origin];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn align_macos_traffic_lights() {}

/// Presents the native macOS `NSSharingServicePicker` anchored to `anchor` (in window points).
/// Returns `true` if presented, or `false` when unavailable so callers can copy the URL instead.
#[cfg(target_os = "macos")]
pub fn show_share_sheet(url: &str, anchor: egui::Rect) -> bool {
    use std::cell::RefCell;
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    thread_local! {
        static ACTIVE_PICKER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    }

    unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![objc2::class!(NSApplication), sharedApplication];
        let Some(app) = app else { return false };
        let key_win: Option<Retained<AnyObject>> = msg_send![&*app, keyWindow];
        let window = match key_win {
            Some(w) => w,
            None => {
                let windows: Option<Retained<AnyObject>> = msg_send![&*app, windows];
                let Some(windows) = windows else { return false };
                let count: usize = msg_send![&*windows, count];
                if count == 0 {
                    return false;
                }
                let Some(w): Option<Retained<AnyObject>> = msg_send![&*windows, objectAtIndex: 0usize] else {
                    return false;
                };
                w
            }
        };
        let content_view: Option<Retained<AnyObject>> = msg_send![&*window, contentView];
        let Some(content_view) = content_view else { return false };

        let ns_str = NSString::from_str(url);
        let ns_url: Option<Retained<AnyObject>> = msg_send![objc2::class!(NSURL), URLWithString: &*ns_str];
        let items: Option<Retained<AnyObject>> = match ns_url.as_deref() {
            Some(u) => msg_send![objc2::class!(NSArray), arrayWithObject: u],
            None => msg_send![objc2::class!(NSArray), arrayWithObject: &*ns_str],
        };
        let Some(items) = items else { return false };

        let picker_alloc: *mut AnyObject = msg_send![objc2::class!(NSSharingServicePicker), alloc];
        if picker_alloc.is_null() {
            return false;
        }
        let picker_ptr: *mut AnyObject = msg_send![picker_alloc, initWithItems: &*items];
        let Some(picker) = Retained::from_raw(picker_ptr) else { return false };

        let bounds: NSRect = msg_send![&*content_view, bounds];
        let flipped: Bool = msg_send![&*content_view, isFlipped];
        let y = if flipped.as_bool() {
            anchor.top() as f64
        } else {
            bounds.size.height - anchor.bottom() as f64
        };
        let rect = NSRect::new(
            NSPoint::new(anchor.left() as f64, y),
            NSSize::new(anchor.width().max(1.0) as f64, anchor.height().max(1.0) as f64),
        );
        // Bottom edge of the button: MaxY (3) when flipped, MinY (1) when unflipped.
        let edge: usize = if flipped.as_bool() { 3 } else { 1 };

        ACTIVE_PICKER.with(|slot| *slot.borrow_mut() = Some(picker.clone()));
        let () = msg_send![&*picker, showRelativeToRect: rect, ofView: &*content_view, preferredEdge: edge];
        true
    }
}

#[cfg(not(target_os = "macos"))]
pub fn show_share_sheet(_url: &str, _anchor: egui::Rect) -> bool {
    false
}

static SYSTEM_ACCENT: std::sync::Mutex<Option<(std::time::Instant, Option<Color32>)>> = std::sync::Mutex::new(None);

/// The system's accent colour (macOS `NSColor.controlAccentColor`, or Windows'
/// `AccentPalette` light-2 / `DWM AccentColor`). Read again every couple of seconds (and
/// invalidated immediately when system appearance/accent changes) so a new OS colour shows
/// without a restart. `None` when unsupported or unavailable.
pub fn system_accent() -> Option<Color32> {
    use std::time::{Duration, Instant};
    let mut cached = SYSTEM_ACCENT.lock_unpoisoned();
    match *cached {
        Some((at, c)) if at.elapsed() < Duration::from_secs(2) => c,
        _ => {
            let c = read_system_accent();
            *cached = Some((Instant::now(), c));
            c
        }
    }
}

#[cfg(target_os = "macos")]
fn read_system_accent() -> Option<Color32> {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    // SAFETY: AppKit `NSColor` / `NSColorSpace` queries return retained/autoreleased
    // catalog objects; convert to sRGB before reading RGB channels.
    unsafe {
        let ns_color: Option<Retained<AnyObject>> = msg_send![objc2::class!(NSColor), controlAccentColor];
        let ns_color = ns_color?;
        let srgb: Option<Retained<AnyObject>> = msg_send![objc2::class!(NSColorSpace), sRGBColorSpace];
        let conv: Option<Retained<AnyObject>> = match srgb.as_deref() {
            Some(cs) => msg_send![&*ns_color, colorUsingColorSpace: cs],
            None => None,
        };
        let c = conv?;
        let r: f64 = msg_send![&*c, redComponent];
        let g: f64 = msg_send![&*c, greenComponent];
        let b: f64 = msg_send![&*c, blueComponent];
        let to_u8 = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Some(Color32::from_rgb(to_u8(r), to_u8(g), to_u8(b)))
    }
}

#[cfg(windows)]
fn read_system_accent() -> Option<Color32> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::w;
    // SAFETY: out-buffers sized to what we pass; the strings are static.
    unsafe {
        // Eight RGBA swatches, lightest first; the second is "light 2".
        let mut palette = [0u8; 32];
        let mut size = palette.len() as u32;
        let key = w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent");
        if RegGetValueW(HKEY_CURRENT_USER, key, w!("AccentPalette"), RRF_RT_REG_BINARY, None, Some(palette.as_mut_ptr() as *mut _), Some(&mut size)).is_ok() && size >= 8 {
            return Some(Color32::from_rgb(palette[4], palette[5], palette[6]));
        }
        // 0xAABBGGRR.
        let mut abgr = 0u32;
        let mut size = 4u32;
        RegGetValueW(HKEY_CURRENT_USER, w!(r"Software\Microsoft\Windows\DWM"), w!("AccentColor"), RRF_RT_REG_DWORD, None, Some(&mut abgr as *mut u32 as *mut _), Some(&mut size))
            .is_ok()
            .then(|| Color32::from_rgb(abgr as u8, (abgr >> 8) as u8, (abgr >> 16) as u8))
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn read_system_accent() -> Option<Color32> {
    None
}

/// Corner radius of album/playlist art in cards.
pub fn card_radius() -> f32 {
    crate::preset::active().app.card_radius.clamp(0.0, 40.0)
}

/// Corner radius of small song-row thumbnails.
pub fn thumb_radius() -> f32 {
    crate::preset::active().app.thumb_radius.clamp(0.0, 20.0)
}

/// Corner radius of panels (settings sections, menus, dialogs).
pub fn panel_radius() -> u8 {
    crate::preset::active().app.panel_radius.clamp(0.0, 32.0) as u8
}

/// Strength of card hover zoom/lift.
pub fn hover_k() -> f32 {
    crate::preset::active().app.hover.clamp(0.0, 2.5)
}

pub fn shade(a: f32) -> Color32 {
    Color32::from_black_alpha((a * 255.0) as u8)
}

/// Scale a colour's opacity by `a` (works for opaque and already-translucent colours).
pub fn with_alpha(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

/// Interpolate in premultiplied space (Color32's native form), so translucent glass
/// colours blend correctly with opaque ones.
pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

// ------------------------------------------------------------------ motion

pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// Every UI transition runs at this fraction of the time it's written with: one knob
/// for how snappy the app feels. (Lyrics keep their own timing.)
pub const SPEED: f32 = 0.65;

/// Animate toward `target` (eased), keyed by `id`. Requests repaints while moving.
pub fn anim(ctx: &egui::Context, id: impl std::hash::Hash + std::fmt::Debug, target: f32, secs: f32) -> f32 {
    ctx.animate_value_with_time(egui::Id::unique(id), target, secs * SPEED)
}

pub fn anim_bool(ctx: &egui::Context, id: impl std::hash::Hash + std::fmt::Debug, on: bool, secs: f32) -> f32 {
    ease_out(ctx.animate_bool_with_time(egui::Id::unique(id), on, secs * SPEED))
}

// ------------------------------------------------------------------ type

pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}
// Named families, made once: these are called hundreds of times a frame, and a
// clone is a reference count instead of a new string.
static SEMIBOLD: std::sync::LazyLock<FontFamily> = std::sync::LazyLock::new(|| FontFamily::Name("semibold".into()));
static BOLD: std::sync::LazyLock<FontFamily> = std::sync::LazyLock::new(|| FontFamily::Name("bold".into()));
static ICONS: std::sync::LazyLock<FontFamily> = std::sync::LazyLock::new(|| FontFamily::Name("icons".into()));

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, SEMIBOLD.clone())
}
pub fn bold(size: f32) -> FontId {
    FontId::new(size, BOLD.clone())
}
pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, ICONS.clone())
}

/// Lucide icons (ISC), subset to the glyphs used here.
pub mod icon {
    pub const LIBRARY: char = '\u{e100}';
    pub const SEARCH: char = '\u{e151}';
    pub const CHEVRON_LEFT: char = '\u{e06e}';
    pub const CHEVRON_RIGHT: char = '\u{e06f}';
    pub const CHEVRON_UP: char = '\u{e070}';
    pub const CHEVRON_DOWN: char = '\u{e06d}';
    pub const PLAY: char = '\u{e13c}';
    pub const PAUSE: char = '\u{e12e}';
    pub const SHUFFLE: char = '\u{e15e}';
    pub const HEART: char = '\u{e0f2}';
    pub const THUMBS_DOWN: char = '\u{e189}';
    pub const LIST_MUSIC: char = '\u{e2e0}';
    pub const X: char = '\u{e1b2}';
    pub const HISTORY: char = '\u{e1f5}';
    pub const TRASH_2: char = '\u{e18e}';
    pub const LIST_PLUS: char = '\u{e23f}';
    pub const LIST_MINUS: char = '\u{e23e}';
    pub const LIST_CHECKS: char = '\u{e1d0}';
    pub const RADIO: char = '\u{e142}';
    pub const DISC_3: char = '\u{e494}';
    pub const LOG_OUT: char = '\u{e10e}';
    pub const MUSIC: char = '\u{e122}';
    pub const SPARKLES: char = '\u{e412}';
    pub const CHECK: char = '\u{e06c}';
    pub const CIRCLE_USER: char = '\u{e461}';
    pub const USERS: char = '\u{e1a4}';
    pub const COPY: char = '\u{e09e}';
    pub const LINK: char = '\u{e102}';
    pub const CROWN: char = '\u{e1d6}';
    pub const HEADPHONES: char = '\u{e0f1}';
    pub const DOWNLOAD: char = '\u{e0b2}';
    pub const FOLDER_OPEN: char = '\u{e247}';
    pub const PIN: char = '\u{e259}';
    pub const PIN_OFF: char = '\u{e2b6}';
    pub const MAXIMIZE_2: char = '\u{e113}';
    pub const SHARE: char = '\u{e19e}';
}

/// Inter (SIL OFL 1.1), the default face: regular / semibold / extra bold, subset to
/// Latin (with Vietnamese), Greek and Cyrillic, so those scripts never fall back to
/// another font letter by letter.
const INTER: [&[u8]; 3] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Inter-ExtraBold.ttf"),
];
const LUCIDE: &[u8] = include_bytes!("../assets/fonts/lucide.ttf");
const NOTO_SINHALA: &[u8] = include_bytes!("../assets/fonts/NotoSansSinhala.ttf");

/// Lyric typefaces used by theme presets, as static regular / semibold / bold
/// instances. They cover Latin only (their upstream has no Vietnamese, Greek or
/// Cyrillic); `lyric_font_for` sets such lines in Inter instead.
/// Instrument Sans is only ever used at semibold (one preset), so its bold slot reuses
/// that instead of shipping a weight nothing draws.
const LYRIC_FONTS: &[(&str, [&[u8]; 3])] = &[
    ("sfpro", [
        include_bytes!("../assets/fonts/SFProDisplay-Regular.otf"),
        include_bytes!("../assets/fonts/SFProDisplay-SemiBold.otf"),
        include_bytes!("../assets/fonts/SFProDisplay-Bold.otf"),
    ]),
    ("dmsans", [
        include_bytes!("../assets/fonts/DMSans-Regular.ttf"),
        include_bytes!("../assets/fonts/DMSans-SemiBold.ttf"),
        include_bytes!("../assets/fonts/DMSans-Bold.ttf"),
    ]),
    ("googlesansflex", [
        include_bytes!("../assets/fonts/GoogleSansFlex-Regular.ttf"),
        include_bytes!("../assets/fonts/GoogleSansFlex-SemiBold.ttf"),
        include_bytes!("../assets/fonts/GoogleSansFlex-Bold.ttf"),
    ]),
    ("figtree", [
        include_bytes!("../assets/fonts/Figtree-Regular.ttf"),
        include_bytes!("../assets/fonts/Figtree-SemiBold.ttf"),
        include_bytes!("../assets/fonts/Figtree-ExtraBold.ttf"),
    ]),
    ("instrumentsans", [
        include_bytes!("../assets/fonts/InstrumentSans-Regular.ttf"),
        include_bytes!("../assets/fonts/InstrumentSans-SemiBold.ttf"),
        include_bytes!("../assets/fonts/InstrumentSans-SemiBold.ttf"),
    ]),
];

/// The bundled font a preset font name refers to. Fonts that can't be bundled
/// (Spotify's) map to the closest open one; unknown names to Inter.
fn lyric_key(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().replace(' ', "").as_str() {
        "sfprodisplay" | "sfpro" => "sfpro",
        "dmsans" => "dmsans",
        "googlesansflex" | "googlesans" => "googlesansflex",
        "figtree" | "spotifymixuititle" | "spotifymix" | "circular" => "figtree",
        "switzer" | "instrumentsans" => "instrumentsans",
        _ => return None,
    })
}

// ------------------------------------------------------------------ font setting

/// The listener's font (Settings › Appearance), used instead of the theme's for the
/// interface and the lyrics. Empty: the theme's own.
static FONT_CHOICE: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

/// An installed font the listener (or the theme) picked, once its faces are loaded. Drawn through the
/// "choice-<weight>" families, which always exist (Inter until something is loaded), so
/// a frame laid out before egui swaps fonts in never names a missing family.
static CHOICE: std::sync::RwLock<Option<Chosen>> = std::sync::RwLock::new(None);

struct Chosen {
    name: String,
    faces: [Face; 3],
}

/// One face of a primary font: the bytes, the face in the file, and the weight to set
/// on a variable font.
#[derive(Clone, Copy)]
struct Face {
    bytes: &'static [u8],
    index: u32,
    wght: Option<f32>,
}

impl Face {
    const fn of(bytes: &'static [u8]) -> Self {
        Self { bytes, index: 0, wght: None }
    }
}

pub fn set_font_choice(name: &str) {
    *FONT_CHOICE.write_unpoisoned() = name.to_owned();
}

pub fn font_choice() -> String {
    FONT_CHOICE.read_unpoisoned().clone()
}

/// The font name to draw with where a theme asks for `theme_font`.
pub fn effective_font(theme_font: &str) -> String {
    let choice = FONT_CHOICE.read_unpoisoned();
    if choice.is_empty() {
        if cfg!(target_os = "macos") && (theme_font.is_empty() || theme_font.eq_ignore_ascii_case("inter")) {
            "sfpro".to_owned()
        } else {
            theme_font.to_owned()
        }
    } else {
        choice.clone()
    }
}

/// The family key for a font name: a bundled face, "choice" for the installed font the
/// listener picked (once loaded), else Inter.
fn font_key(name: &str) -> &'static str {
    let name = effective_font(name);
    if let Some(k) = lyric_key(&name) {
        return k;
    }
    if CHOICE.read_unpoisoned().as_ref().is_some_and(|c| c.name.eq_ignore_ascii_case(&name)) { "choice" } else { "inter" }
}

fn family(key: &str, weight: crate::preset::Weight, size: f32) -> FontId {
    use crate::preset::Weight;
    let w = match weight {
        Weight::Regular => 0,
        Weight::Semibold => 1,
        Weight::Bold => 2,
    };
    FontId::new(size, FontFamily::Name(format!("{key}-{w}").into()))
}

/// Lyric font for a preset's font name and weight.
pub fn lyric_font(name: &str, weight: crate::preset::Weight, size: f32) -> FontId {
    family(font_key(name), weight, size)
}

/// Lyric font for one piece of text. When the font lacks letters the text uses and
/// Inter has them (Vietnamese, Greek or Cyrillic in DM Sans, Figtree, …), the whole
/// text is set in Inter rather than mixing two typefaces letter by letter.
pub fn lyric_font_for(name: &str, weight: crate::preset::Weight, size: f32, text: &str) -> FontId {
    let key = font_key(name);
    let key = if key != "inter" && lacks_letters(key, text) { "inter" } else { key };
    family(key, weight, size)
}

fn lacks_letters(key: &str, text: &str) -> bool {
    use skrifa::MetadataProvider;
    let face = match key {
        "choice" => match CHOICE.read_unpoisoned().as_ref() {
            Some(c) => c.faces[0],
            None => return false,
        },
        k => match LYRIC_FONTS.iter().find(|(n, _)| *n == k) {
            Some((_, w)) => Face::of(w[0]),
            None => return false,
        },
    };
    let (Ok(font), Ok(inter)) = (skrifa::FontRef::from_index(face.bytes, face.index), skrifa::FontRef::new(INTER[0])) else { return false };
    let (has, inter) = (font.charmap(), inter.charmap());
    text.chars().any(|c| c.is_alphabetic() && has.map(c).is_none() && inter.map(c).is_some())
}

// ------------------------------------------------------------------ installed fonts

/// A font family installed on this PC, for the font setting.
pub struct SysFamily {
    pub name: String,
    faces: Vec<SysFace>,
}

struct SysFace {
    path: std::path::PathBuf,
    index: u32,
    weight: f32,
    /// The weight axis range, for a variable font.
    wght: Option<(f32, f32)>,
}

static INSTALLED_FONTS: std::sync::OnceLock<Vec<SysFamily>> = std::sync::OnceLock::new();
fn font_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    #[cfg(target_os = "windows")]
    {
        let win = std::env::var_os("WINDIR")
            .or_else(|| std::env::var_os("SystemRoot"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
        dirs.push(win.join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(std::path::PathBuf::from(local).join("Microsoft/Windows/Fonts"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        dirs.push("/System/Library/Fonts".into());
        dirs.push("/Library/Fonts".into());
        if let Some(home) = std::env::var_os("HOME") {
            let home = std::path::PathBuf::from(home);
            dirs.push(home.join("Library/Fonts"));
            dirs.push(home.join(".local/share/fonts"));
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        dirs.push("/usr/share/fonts".into());
        dirs.push("/usr/local/share/fonts".into());
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(std::path::PathBuf::from(home).join(".local/share/fonts"));
        }
    }
    dirs
}

/// Every upright, normal-width face of a text font (one that maps Latin letters), by
/// family. Only names and attributes are read; nothing stays mapped.
fn scan_fonts() -> Vec<SysFamily> {
    use skrifa::{MetadataProvider, attribute::Style, string::StringId};
    let mut files = Vec::new();
    let mut dirs = font_dirs();
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().and_then(|x| x.to_str()).is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc")) {
                files.push(path);
            }
        }
    }
    let mut families: std::collections::BTreeMap<String, SysFamily> = std::collections::BTreeMap::new();
    for path in files {
        let Ok(file) = std::fs::File::open(&path) else { continue };
        // SAFETY: read-only and dropped before the next file; font files don't change
        // under a running app.
        let Ok(data) = (unsafe { memmap2::Mmap::map(&file) }) else { continue };
        let Ok(fonts) = skrifa::raw::FileRef::new(&data) else { continue };
        for (index, font) in fonts.fonts().enumerate() {
            let Ok(font) = font else { continue };
            let cmap = font.charmap();
            if cmap.is_symbol() || cmap.map('a').is_none() || cmap.map('Z').is_none() {
                continue;
            }
            let attrs = font.attributes();
            if attrs.style != Style::Normal || (attrs.stretch.ratio() - 1.0).abs() > 0.01 {
                continue;
            }
            let name = [StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME]
                .into_iter()
                .find_map(|id| font.localized_strings(id).english_or_first())
                .map(|s| s.to_string().trim().to_owned());
            // "@" names are vertical CJK variants.
            let Some(name) = name.filter(|n| !n.is_empty() && !n.starts_with('@') && lyric_key(n).is_none() && !n.eq_ignore_ascii_case("inter")) else { continue };
            let wght = font.axes().iter().find(|a| a.tag() == skrifa::Tag::new(b"wght")).map(|a| (a.min_value(), a.max_value()));
            let face = SysFace { path: path.clone(), index: index as u32, weight: attrs.weight.value(), wght };
            families.entry(name.to_lowercase()).or_insert_with(|| SysFamily { name, faces: Vec::new() }).faces.push(face);
        }
    }
    families.into_values().collect()
}

/// The closest face to each of the three weights the app draws: regular, semibold, and
/// bold, leaning to 700 over 900 where a family has both.
fn choose_faces(family: &SysFamily) -> Option<[Face; 3]> {
    let mut faces = Vec::new();
    for target in [400.0, 600.0, 750.0] {
        let best = family.faces.iter().min_by_key(|f| match f.wght {
            Some((lo, hi)) if (lo..=hi).contains(&target) => 0,
            Some((lo, hi)) => (target - target.clamp(lo, hi)).abs() as i32,
            None => (f.weight - target).abs() as i32,
        })?;
        let bytes = map(&best.path)?;
        faces.push(Face { bytes, index: best.index, wght: best.wght.map(|(lo, hi)| target.clamp(lo, hi)) });
    }
    faces.try_into().ok()
}

/// Load the faces of the installed font in use, if that's what it is: the listener's
/// pick, else one the theme names for its interface (Fluent's Segoe UI Variable). A
/// name can list fallbacks, "Segoe UI Variable, Segoe UI", tried in order.
fn load_choice(ui_font: &str) {
    let name = effective_font(ui_font);
    if name.is_empty() || lyric_key(&name).is_some() || name.eq_ignore_ascii_case("inter") {
        *CHOICE.write_unpoisoned() = None;
        return;
    }
    if CHOICE.read_unpoisoned().as_ref().is_some_and(|c| c.name == name) {
        return;
    }
    // Only when an installed font is set: a one-off scan, the first time.
    let all = INSTALLED_FONTS.get_or_init(scan_fonts);
    let faces = name.split(',').map(str::trim).find_map(|n| all.iter().find(|f| f.name.eq_ignore_ascii_case(n)).and_then(choose_faces));
    if faces.is_none() {
        log::warn!("font {name:?} isn't installed; using Inter");
    }
    *CHOICE.write_unpoisoned() = faces.map(|faces| Chosen { name, faces });
}

// System fallbacks: egui's fonts cover Latin only, so CJK, Arabic, Sinhala,
// symbols and emoji would otherwise render as rectangle boxes. Memory-mapped, not read,
// so fallback collections cost only the glyph pages actually drawn.
#[cfg(target_os = "macos")]
fn system_fallbacks() -> [Vec<(String, Face)>; 3] {
    const SINHALA: &str = "/System/Library/Fonts/Supplemental/Sinhala Sangam MN.ttc";
    const CANDIDATES: &[(&str, u32)] = &[
        ("/System/Library/Fonts/Supplemental/Arial Unicode.ttf", 0),
        ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
        ("/System/Library/Fonts/AppleSDGothicNeo.ttc", 0),
        (SINHALA, 0),
        ("/System/Library/Fonts/Apple Symbols.ttf", 0),
    ];
    const HEAVIER: &[(&str, [(&str, u32); 2])] = &[(SINHALA, [(SINHALA, 1), (SINHALA, 1)])];
    std::array::from_fn(|w| {
        CANDIDATES
            .iter()
            .filter_map(|&(path, index)| {
                let heavy = (w > 0).then(|| HEAVIER.iter().find(|(base, _)| *base == path)).flatten().map(|(_, v)| v[w - 1]);
                let mapped = |(p, i): (&str, u32)| validated_face(p, i, w);
                heavy.and_then(mapped).or_else(|| mapped((path, index)))
            })
            .collect()
    })
}

#[cfg(target_os = "windows")]
fn system_fallbacks() -> [Vec<(String, Face)>; 3] {
    let dirs = font_dirs();
    let find_in_dirs = |names: &[&str]| -> Option<std::path::PathBuf> {
        names.iter().find_map(|name| dirs.iter().map(|d| d.join(name)).find(|p| p.is_file()))
    };
    let mut entries: Vec<[(Option<std::path::PathBuf>, u32); 3]> = Vec::new();
    // Sinhala first: static Iskoola Pota TTF (when installed), then Nirmala UI (built-in
    // Windows Indic/Sinhala superfont, supporting both TTC and single-TTF/variable builds).
    let iskpota_reg = find_in_dirs(&["iskpota.ttf"]);
    let iskpota_bold = find_in_dirs(&["iskpotab.ttf"]).or_else(|| iskpota_reg.clone());
    let mut has_sinhala = false;
    if let Some(reg) = iskpota_reg {
        entries.push([(Some(reg), 0), (iskpota_bold.clone(), 0), (iskpota_bold, 0)]);
        has_sinhala = true;
    }
    if let Some(nirmala) = find_in_dirs(&["Nirmala.ttc"]) {
        entries.push([(Some(nirmala.clone()), 0), (Some(nirmala.clone()), 1), (Some(nirmala), 1)]);
        has_sinhala = true;
    } else if let Some(nirmala_ttf) = find_in_dirs(&["Nirmala.ttf"]) {
        let bold = find_in_dirs(&["NirmalaB.ttf"]).or_else(|| Some(nirmala_ttf.clone()));
        entries.push([(Some(nirmala_ttf), 0), (bold.clone(), 0), (bold, 0)]);
        has_sinhala = true;
    }
    // Additional CJK, symbol, and emoji system fallbacks on Windows.
    for (reg_names, bold_names) in [
        (&["seguisym.ttf"][..], &["seguisym.ttf"][..]),
        (&["msyh.ttc"][..], &["msyhbd.ttc", "msyh.ttc"][..]),
        (&["YuGothR.ttc", "msgothic.ttc"][..], &["YuGothB.ttc", "msgothic.ttc"][..]),
        (&["malgun.ttf"][..], &["malgunbd.ttf", "malgun.ttf"][..]),
        (&["seguiemj.ttf"][..], &["seguiemj.ttf"][..]),
    ] {
        if let Some(reg) = find_in_dirs(reg_names) {
            let bold = find_in_dirs(bold_names).or_else(|| Some(reg.clone()));
            entries.push([(Some(reg), 0), (bold.clone(), 0), (bold, 0)]);
        }
    }
    // If neither Iskoola Pota nor Nirmala was found, scan Windows font folders for any
    // other Sinhala font (e.g. Noto Sans Sinhala, Abhaya Libre) across all weights.
    let sinhala_extra = if has_sinhala { Vec::new() } else { scan_sinhala_fonts(&dirs) };
    std::array::from_fn(|w| {
        let mut out: Vec<(String, Face)> = entries
            .iter()
            .filter_map(|slots| {
                let (path, idx) = (&slots[w].0, slots[w].1);
                path.as_ref().and_then(|p| validated_face(p, idx, w))
                    .or_else(|| slots[0].0.as_ref().and_then(|p| validated_face(p, slots[0].1, w)))
            })
            .collect();
        for (p, idx) in &sinhala_extra {
            if let Some(item) = validated_face(p, *idx, w)
                && !out.iter().any(|(id, _)| *id == item.0)
            {
                out.push(item);
            }
        }
        out
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn system_fallbacks() -> [Vec<(String, Face)>; 3] {
    let extra = scan_sinhala_fonts(&font_dirs());
    std::array::from_fn(|w| extra.iter().filter_map(|(p, idx)| validated_face(p, *idx, w)).collect())
}

fn validated_face(path: impl AsRef<std::path::Path>, index: u32, weight_slot: usize) -> Option<(String, Face)> {
    use skrifa::MetadataProvider;
    let p = path.as_ref();
    let bytes = map(p)?;
    let font = skrifa::FontRef::from_index(bytes, index).ok()?;
    let target = [400.0f32, 600.0, 750.0][weight_slot.min(2)];
    let wght = font
        .axes()
        .iter()
        .find(|a| a.tag() == skrifa::Tag::new(b"wght"))
        .map(|a| target.clamp(a.min_value(), a.max_value()));
    Some((format!("sys:{}#{index}", p.display()), Face { bytes, index, wght }))
}

#[cfg(not(target_os = "macos"))]
fn scan_sinhala_fonts(dirs: &[std::path::PathBuf]) -> Vec<(std::path::PathBuf, u32)> {
    use skrifa::MetadataProvider;
    let mut found = Vec::new();
    let mut stack = dirs.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let is_font = path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc"));
            if !is_font {
                continue;
            }
            let Ok(file) = std::fs::File::open(&path) else { continue };
            let Ok(data) = (unsafe { memmap2::Mmap::map(&file) }) else { continue };
            let Ok(fonts) = skrifa::raw::FileRef::new(&data) else { continue };
            if let Some((idx, _)) = fonts.fonts().enumerate().find(|(_, f)| {
                f.as_ref().is_ok_and(|font| font.charmap().map('\u{0D85}').is_some())
            }) {
                found.push((path, idx as u32));
            }
        }
    }
    found
}

/// Ascent and line height in em, as egui lays text out (skrifa's metrics: OS/2 typo
/// values when the font asks for them, else hhea).
fn em_metrics(bytes: &[u8], index: u32) -> Option<(f32, f32)> {
    use skrifa::MetadataProvider;
    let font = skrifa::FontRef::from_index(bytes, index).ok()?;
    let m = font.metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default());
    let upem = m.units_per_em.max(1) as f32;
    Some((m.ascent / upem, (m.ascent - m.descent + m.leading) / upem))
}

fn font_data(face: Face, y_offset_factor: f32) -> FontData {
    let mut tweak = egui::FontTweak { y_offset_factor, ..Default::default() };
    if let Some(w) = face.wght {
        tweak.coords.push(b"wght", w);
    }
    FontData { index: face.index, ..FontData::from_static(face.bytes) }.tweak(tweak)
}

/// Install fonts. `ui_font` is the theme's font name for all interface text ("inter"
/// or one of the bundled lyric fonts); the listener's font setting wins over it.
///
/// Each primary font is followed by Inter at the same weight (for the scripts the
/// theme fonts lack), then the system fallbacks at the closest weight.
///
/// egui places a fallback font's glyphs by centring its line height on the primary
/// font's, not by sharing a baseline, so CJK glyphs (Yu Gothic, Microsoft YaHei, …)
/// sit visibly higher than Latin ones in the same line. Each fallback therefore gets a
/// y offset computed from both fonts' metrics, once per primary font it backs.
pub fn install_fonts(ctx: &egui::Context, ui_font: &str) {
    load_choice(ui_font);
    let ui_key = font_key(ui_font);
    // Installing rebuilds the font atlas; most themes share a UI font, so skip repeats.
    static INSTALLED: std::sync::Mutex<Option<(&'static str, String)>> = std::sync::Mutex::new(None);
    let loaded = CHOICE.read_unpoisoned().as_ref().map(|c| c.name.clone()).unwrap_or_default();
    let state = Some((ui_key, loaded));
    {
        let mut installed = INSTALLED.lock_unpoisoned();
        if *installed == state {
            return;
        }
        *installed = state;
    }
    let mut defs = FontDefinitions::default();
    let egui_default = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    defs.font_data.insert("lucide".into(), Arc::new(FontData::from_static(LUCIDE).tweak(egui::FontTweak { y_offset_factor: 0.0, ..Default::default() })));
    defs.families.insert(FontFamily::Name("icons".into()), vec!["lucide".to_owned()]);

    // Mapped once for the life of the process, however often fonts are installed:
    // the fallback list at each weight.
    type Fallbacks = [Vec<(String, Face)>; 3];
    static SYSTEM: std::sync::OnceLock<Fallbacks> = std::sync::OnceLock::new();
    let system = SYSTEM.get_or_init(system_fallbacks);
    // Primary fonts: Inter, each lyric font, and the installed font picked in Settings.
    let choice = CHOICE.read_unpoisoned().as_ref().map(|c| c.faces);
    let mut groups: Vec<(&str, [Face; 3])> = vec![("inter", INTER.map(Face::of))];
    groups.extend(LYRIC_FONTS.iter().map(|(k, w)| (*k, w.map(Face::of))));
    groups.extend(choice.map(|faces| ("choice", faces)));
    for (key, faces) in &groups {
        let (asc_p, h_p) = em_metrics(faces[0].bytes, faces[0].index).unwrap_or((0.97, 1.21));
        for (w, face) in faces.iter().enumerate() {
            let name = format!("{key}-{w}");
            defs.font_data.insert(name.clone(), Arc::new(font_data(*face, 0.0)));
            let mut chain = vec![name.clone()];
            let inter = (*key != "inter").then(|| ("inter".to_owned(), Face::of(INTER[w])));
            let sinhala = Some((
                "noto-sinhala".to_owned(),
                Face { bytes: NOTO_SINHALA, index: 0, wght: Some([400.0, 600.0, 750.0][w]) },
            ));
            for (fallback, f) in inter.iter().chain(&sinhala).chain(&system[w]) {
                let Some((asc_f, h_f)) = em_metrics(f.bytes, f.index) else { continue };
                let shift = asc_p - (asc_f + 0.5 * (h_p - h_f));
                let id = format!("{fallback}-{w}@{key}");
                if !defs.font_data.contains_key(&id) {
                    defs.font_data.insert(id.clone(), Arc::new(font_data(*f, shift)));
                }
                chain.push(id);
            }
            if *key == "inter" && w == 0
                && let Some(mono) = defs.families.get_mut(&FontFamily::Monospace)
            {
                mono.extend(chain[1..].iter().cloned());
            }
            // egui's bundled emoji/icon font stays as a last resort.
            chain.extend(egui_default.iter().filter(|n| n.as_str() != "Ubuntu-Light").cloned());
            defs.families.insert(FontFamily::Name(name.clone().into()), chain.clone());
            if *key == ui_key {
                let family = match w {
                    0 => FontFamily::Proportional,
                    1 => FontFamily::Name("semibold".into()),
                    _ => FontFamily::Name("bold".into()),
                };
                defs.families.insert(family, chain);
            }
        }
    }
    // With no installed font picked, the "choice" families are Inter's, so they always
    // exist without a second copy of Inter and its fallbacks.
    if choice.is_none() {
        for w in 0..3 {
            let inter = defs.families[&FontFamily::Name(format!("inter-{w}").into())].clone();
            defs.families.insert(FontFamily::Name(format!("choice-{w}").into()), inter);
        }
    }
    // The same bytes get the same `Arc` on every install, so comparing these definitions
    // with the installed ones is a pointer check per font instead of reading every file.
    for data in defs.font_data.values_mut() {
        let font = shared_bytes(&data.font);
        if !Arc::ptr_eq(&font, &data.font) {
            *data = Arc::new(FontData { font, ..(**data).clone() });
        }
    }
    ctx.set_fonts(defs);
}

/// The first `Blob` seen for these bytes. Font bytes here are all `'static` (bundled, or
/// mapped for the rest of the process), so the address identifies them for good.
fn shared_bytes(blob: &egui::epaint::text::Blob) -> egui::epaint::text::Blob {
    static SEEN: std::sync::Mutex<Vec<egui::epaint::text::Blob>> = std::sync::Mutex::new(Vec::new());
    let bytes = (**blob).as_ref();
    let mut seen = SEEN.lock_unpoisoned();
    if let Some(b) = seen.iter().find(|b| std::ptr::eq((***b).as_ref(), bytes)) {
        return b.clone();
    }
    seen.push(blob.clone());
    blob.clone()
}

/// Map a font file for the rest of the process. Kept by canonical path, so a file is
/// mapped once however many weights, faces or installed-font picks use it, and whichever
/// spelling names it (the installed-font scan's C:\WINDOWS, the fallback list's C:\Windows).
fn map(path: impl AsRef<std::path::Path>) -> Option<&'static [u8]> {
    static MAPPED: std::sync::Mutex<Vec<(std::path::PathBuf, &'static [u8])>> = std::sync::Mutex::new(Vec::new());
    let path = std::fs::canonicalize(path).ok()?;
    let mut mapped = MAPPED.lock_unpoisoned();
    if let Some((_, bytes)) = mapped.iter().find(|(p, _)| *p == path) {
        return Some(bytes);
    }
    let file = std::fs::File::open(&path).ok()?;
    // SAFETY: system font files aren't modified while in use; the mapping is
    // read-only and intentionally lives for the rest of the process.
    let mmap = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    let bytes: &'static [u8] = Box::leak(Box::new(mmap));
    mapped.push((path, bytes));
    Some(bytes)
}

/// Apply a theme preset's chrome: how strong glass surfaces are and how round
/// buttons, pills and fields are.
pub fn apply_chrome(ctx: &egui::Context, p: &crate::preset::Preset) {
    GLASS.store(p.chrome.glass.clamp(0.0, 2.5).to_bits(), std::sync::atomic::Ordering::Relaxed);
    let dim = crate::preset::hex(&p.app.text_dim).unwrap_or(TEXT_DIM);
    DIM.store(u32::from_le_bytes(dim.to_array()), std::sync::atomic::Ordering::Relaxed);
    install_style(ctx);
    install_fonts(ctx, &p.app.font);
}

pub fn install_style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(14.0, 7.0);
        s.spacing.menu_margin = egui::Margin::symmetric(4, 5);
        s.spacing.scroll = egui::style::ScrollStyle::floating();
        // A very small give at the ends of every scroller (vendor/egui sets how small:
        // 5% of the view, a light pull), then a quick spring back. Coasting stays.
        s.spacing.scroll.kinetic.rubber_band = true;
        s.text_styles.insert(egui::TextStyle::Body, regular(14.0));
        s.text_styles.insert(egui::TextStyle::Button, semibold(14.0));
        s.text_styles.insert(egui::TextStyle::Heading, bold(22.0));
        s.text_styles.insert(egui::TextStyle::Small, regular(12.0));
        s.visuals = visuals();
        s.interaction.selectable_labels = false;
        s.animation_time = 0.18 * SPEED;
    });
}

/// egui's colours and shapes for the current mode and the active preset's chrome.
fn visuals() -> egui::Visuals {
    let p = crate::preset::active();
    let mut v = if light() { egui::Visuals::light() } else { egui::Visuals::dark() };
    v.panel_fill = bg();
    v.window_fill = menu_fill();
    v.extreme_bg_color = glass(0.08);
    v.override_text_color = Some(text());
    v.selection.bg_fill = p.accent(RED);
    v.hyperlink_color = text();
    v.interact_cursor = Some(egui::CursorIcon::PointingHand);
    v.window_corner_radius = egui::CornerRadius::same(10);
    // The built-in look's menus are shaped like the system's with frosted glass corners.
    v.menu_corner_radius = egui::CornerRadius::same(10);
    v.window_stroke = if light() {
        egui::Stroke::new(1.0, Color32::from_black_alpha(35))
    } else {
        egui::Stroke::new(1.0, glass(0.14))
    };
    v.popup_shadow = if light() {
        egui::epaint::Shadow { offset: [0, 6], blur: 24, spread: 1, color: Color32::from_black_alpha(45) }
    } else {
        egui::epaint::Shadow { offset: [0, 8], blur: 30, spread: 1, color: Color32::from_black_alpha(160) }
    };
    let r = egui::CornerRadius::same(p.chrome.radius.clamp(2.0, 28.0) as u8);
    for (w, fill) in [
        (&mut v.widgets.inactive, glass(0.08)),
        (&mut v.widgets.hovered, glass(0.14)),
        (&mut v.widgets.active, glass(0.2)),
        (&mut v.widgets.open, glass(0.14)),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = egui::Stroke::NONE;
        w.corner_radius = r;
        w.fg_stroke.color = text();
    }
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, glass(0.08));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_macos_vibrancy_constants() {
        #[cfg(target_os = "macos")]
        {
            use macos_vibrancy::*;
            assert_eq!(NS_VISUAL_EFFECT_MATERIAL_UNDER_WINDOW_BACKGROUND, 21);
            assert_eq!(NS_VISUAL_EFFECT_BLENDING_MODE_BEHIND_WINDOW, 0);
            assert_eq!(NS_VISUAL_EFFECT_STATE_FOLLOWS_WINDOW_ACTIVE_STATE, 0);
            assert_eq!(NS_VIEW_WIDTH_SIZABLE, 2);
            assert_eq!(NS_VIEW_HEIGHT_SIZABLE, 16);
            assert_eq!(NS_VIEW_AUTORESIZING_MASK, 18);
            assert_eq!(NS_WINDOW_BELOW, -1);
        }
    }

    #[test]
    fn test_apply_native_vibrancy_does_not_panic() {
        // In test environments where NSApplication may have zero windows or headless execution,
        // apply_native_vibrancy must exit gracefully without panicking.
        apply_native_vibrancy();
    }

    #[test]
    fn bundled_noto_sinhala_covers_sinhala_letters_and_metrics() {
        use skrifa::MetadataProvider;
        let font = skrifa::FontRef::from_index(NOTO_SINHALA, 0).expect("valid NotoSansSinhala TTF");
        assert!(font.charmap().map('\u{0D85}').is_some(), "missing Sinhala letter අ");
        assert!(font.charmap().map('\u{0DC3}').is_some(), "missing Sinhala letter ස");
        assert!(em_metrics(NOTO_SINHALA, 0).is_some());
    }

    #[test]
    fn system_accent_reads_without_panicking() {
        let accent = system_accent();
        #[cfg(target_os = "macos")]
        assert!(accent.is_some(), "macOS NSColor.controlAccentColor should resolve");
        #[cfg(not(target_os = "macos"))]
        let _ = accent;
    }
}


