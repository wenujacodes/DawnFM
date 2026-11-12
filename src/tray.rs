//! System tray icon, shown while "Close to tray" has hidden the window.
//!
//! The icon must be created on the UI thread (it owns a Win32 message window), so
//! the app creates and drops it from `logic`. Clicks arrive on that thread's message
//! loop; they are queued here and the UI is woken to handle them. On macOS it is a
//! menu bar item, made the same way (AppKit wants it on the main thread, which is the
//! UI thread there).
//!
//! On Linux the icon is a GTK one, and GTK runs its own loop on one thread for the
//! life of the process (it can't be started twice); the icon is made and dropped there.

use crate::sync::MutexExt;
use std::sync::{Mutex, Once};

use tray_icon::menu::{Menu, MenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Show,
    Quit,
}

static PENDING: Mutex<Vec<Cmd>> = Mutex::new(Vec::new());

pub struct Tray {
    _icon: TrayIcon,
}

impl Tray {
    /// `wake` is called (from the UI thread's message loop) after each click.
    pub fn new(wake: impl Fn() + Send + Sync + Clone + 'static) -> anyhow::Result<Self> {
        // The handlers are process-wide and can only be set once.
        static HANDLERS: Once = Once::new();
        HANDLERS.call_once(|| {
            let w = wake.clone();
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                    push(Cmd::Show, &w);
                }
            }));
            // Its menu's Show and Quit go through the menu bar's handler (`menubar`).
            let _ = &wake;
        });
        Ok(Self { _icon: build(template_icon()?)? })
    }
}

fn build(png: image::RgbaImage) -> anyhow::Result<TrayIcon> {
    let menu = Menu::with_items(&[
        &MenuItem::with_id("show", "Show Dusk FM", true, None),
        &MenuItem::with_id("quit", "Quit", true, None),
    ])?;
    let (w, h) = png.dimensions();
    Ok(TrayIconBuilder::new()
        // Drawn by macOS in the menu bar's own colour, like other menu bar icons.
        .with_icon_templated(tray_icon::Icon::from_rgba(png.into_raw(), w, h)?)
        .with_tooltip("Dusk FM")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()?)
}

/// The app's mark (a setting sun), centred on a square for the menu bar.
fn template_icon() -> anyhow::Result<image::RgbaImage> {
    const MARK: &[u8] = include_bytes!("../assets/icon/mark.png");
    let mark = image::load_from_memory(MARK)?.into_rgba8();
    let side = 44;
    let fit = image::imageops::resize(&mark, side, side * mark.height() / mark.width(), image::imageops::FilterType::Lanczos3);
    let mut out = image::RgbaImage::new(side, side);
    image::imageops::overlay(&mut out, &fit, 0, i64::from((side - fit.height()) / 2));
    Ok(out)
}

pub(crate) fn push(cmd: Cmd, wake: &impl Fn()) {
    PENDING.lock_unpoisoned().push(cmd);
    wake();
}

/// Clicks since the last call.
pub fn take() -> Vec<Cmd> {
    std::mem::take(&mut *PENDING.lock_unpoisoned())
}
