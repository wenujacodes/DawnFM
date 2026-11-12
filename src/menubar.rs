//! The macOS menu bar: Dawn FM, Controls, View, Window and Help, laid out the way
//! Mac apps (Music among them) lay them out. Clicks are queued here and the UI is
//! woken to handle them, as with the tray and the Dock menu.

use std::sync::Mutex;

use tray_icon::menu::accelerator::{Accelerator, Code, Modifiers};
use tray_icon::menu::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::sync::MutexExt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Settings,
    Quit,
    Toggle,
    Next,
    Prev,
    VolumeUp,
    VolumeDown,
    Lyrics,
    UpNext,
    Mini,
    Sidebar,
}

static PENDING: Mutex<Vec<Cmd>> = Mutex::new(Vec::new());

const REPO: &str = "https://github.com/wenujacodes/DawnFM";

/// Menu ids, and what each does.
const ITEMS: &[(&str, Cmd)] = &[
    ("mb-settings", Cmd::Settings),
    ("mb-quit", Cmd::Quit),
    ("mb-toggle", Cmd::Toggle),
    ("mb-next", Cmd::Next),
    ("mb-prev", Cmd::Prev),
    ("mb-vol-up", Cmd::VolumeUp),
    ("mb-vol-down", Cmd::VolumeDown),
    ("mb-lyrics", Cmd::Lyrics),
    ("mb-queue", Cmd::UpNext),
    ("mb-mini", Cmd::Mini),
    ("mb-sidebar", Cmd::Sidebar),
];

/// Clicks since the last call.
pub fn take() -> Vec<Cmd> {
    std::mem::take(&mut *PENDING.lock_unpoisoned())
}

fn item(id: &str, text: &str, mods: Modifiers, key: Option<Code>) -> MenuItem {
    MenuItem::with_id(id, text, true, key.map(|k| Accelerator::new(mods, k)))
}

/// Build the menu bar and route its clicks. Call once, on the main thread, once the
/// app has finished launching. Also takes over the menu click handler for the
/// tray's Show and Quit (there is one handler per process).
pub fn install(wake: impl Fn() + Send + Sync + Clone + 'static) {
    let cmd = Modifiers::META;
    let version = env!("CARGO_PKG_VERSION");
    let about = AboutMetadata {
        name: Some("Dawn FM".into()),
        version: Some(version.into()),
        short_version: Some(version.into()),
        copyright: Some("GPL-3.0 · Based on BitChord by jackychan4891".into()),
        ..Default::default()
    };
    let sep = PredefinedMenuItem::separator;

    let app = Submenu::new("Dawn FM", true);
    let controls = Submenu::new("Controls", true);
    let view = Submenu::new("View", true);
    let window = Submenu::new("Window", true);
    let help = Submenu::new("Help", true);

    let built = (|| -> tray_icon::menu::Result<Menu> {
        app.append_items(&[
            &PredefinedMenuItem::about(Some("About Dawn FM"), Some(about)),
            &sep(),
            &item("mb-settings", "Settings…", cmd, Some(Code::Comma)),
            &sep(),
            &PredefinedMenuItem::services(None),
            &sep(),
            &PredefinedMenuItem::hide(Some("Hide Dawn FM")),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &sep(),
            // Ours, not AppKit's terminate: the app saves its state on the way out.
            &item("mb-quit", "Quit Dawn FM", cmd, Some(Code::KeyQ)),
        ])?;
        // Space stays the app's own key: as a menu shortcut it would swallow spaces
        // typed into the search box.
        controls.append_items(&[
            &item("mb-toggle", "Play / Pause", cmd, None),
            &item("mb-next", "Next", cmd, Some(Code::ArrowRight)),
            &item("mb-prev", "Previous", cmd, Some(Code::ArrowLeft)),
            &sep(),
            &item("mb-vol-up", "Increase Volume", cmd, Some(Code::ArrowUp)),
            &item("mb-vol-down", "Decrease Volume", cmd, Some(Code::ArrowDown)),
        ])?;
        view.append_items(&[
            &item("mb-lyrics", "Lyrics", cmd, Some(Code::KeyL)),
            &item("mb-queue", "Up Next", cmd, None),
            &item("mb-sidebar", "Show or Hide Sidebar", cmd | Modifiers::CONTROL, Some(Code::KeyS)),
            &sep(),
            &item("mb-mini", "Mini Player", cmd | Modifiers::ALT, Some(Code::KeyM)),
            &PredefinedMenuItem::fullscreen(None),
        ])?;
        window.append_items(&[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(Some("Zoom")),
            &sep(),
            &PredefinedMenuItem::bring_all_to_front(None),
        ])?;
        help.append_items(&[
            &MenuItem::with_id("mb-site", "Dawn FM on GitHub", true, None),
            &MenuItem::with_id("mb-issue", "Report a Problem…", true, None),
        ])?;
        Menu::with_items(&[&app, &controls, &view, &window, &help])
    })();
    let menu = match built {
        Ok(m) => m,
        Err(e) => return log::warn!("menubar: couldn't build the menu: {e}"),
    };
    menu.init_for_nsapp();
    window.set_as_windows_menu_for_nsapp();
    help.set_as_help_menu_for_nsapp();
    // AppKit keeps showing the menu; it must outlive this call.
    std::mem::forget(menu);

    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let id = e.id.as_ref();
        match id {
            "show" => crate::tray::push(crate::tray::Cmd::Show, &wake),
            "quit" => crate::tray::push(crate::tray::Cmd::Quit, &wake),
            "mb-site" => open(REPO),
            "mb-issue" => open(&format!("{REPO}/issues/new")),
            _ => {
                if let Some((_, c)) = ITEMS.iter().find(|(i, _)| *i == id) {
                    PENDING.lock_unpoisoned().push(*c);
                    wake();
                }
            }
        }
    }));
    log::info!("menubar: installed");
}

fn open(url: &str) {
    let _ = std::process::Command::new("open").arg(url).spawn();
}

#[cfg(test)]
mod tests {
    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<_> = super::ITEMS.iter().map(|(i, _)| *i).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), super::ITEMS.len());
    }
}
