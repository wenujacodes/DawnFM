//! The Dock icon's right-click menu: the current song, Play/Pause, Next, Previous.
//!
//! AppKit asks the application delegate for it (`applicationDockMenu:`), and the
//! delegate is winit's, which doesn't implement that. `install` adds the method to the
//! delegate's class at runtime once the app is up. The menu's items target a small
//! object of ours; a click queues a `Cmd` here and wakes the UI, which drains the
//! queue with `take()` (the same pattern as `tray.rs`).

use crate::sync::MutexExt;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Toggle,
    Next,
    Prev,
}

static PENDING: Mutex<Vec<Cmd>> = Mutex::new(Vec::new());

type Waker = Box<dyn Fn() + Send>;
static WAKE: Mutex<Option<Waker>> = Mutex::new(None);

/// What the menu shows: the song's title (a disabled header) and whether to offer Pause or Play.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State {
    title: Option<String>,
    playing: bool,
}

static STATE: Mutex<State> = Mutex::new(State { title: None, playing: false });

/// Keep the menu in step with the player. Cheap enough to call every frame: it only
/// copies the title when it changes.
pub fn set_state(title: Option<&str>, playing: bool) {
    let mut s = STATE.lock_unpoisoned();
    s.playing = playing;
    if s.title.as_deref() != title {
        s.title = title.map(str::to_owned);
    }
}

fn push(cmd: Cmd) {
    PENDING.lock_unpoisoned().push(cmd);
    if let Some(wake) = WAKE.lock_unpoisoned().as_ref() {
        wake();
    }
}

/// Clicks since the last call, oldest first.
pub fn take() -> Vec<Cmd> {
    std::mem::take(&mut *PENDING.lock_unpoisoned())
}

/// The Pause/Play item's title.
fn toggle_label(playing: bool) -> &'static str {
    if playing { "Pause" } else { "Play" }
}

/// Give the Dock icon its menu, waking the UI with `wake` after each click. Call on the
/// main thread once the event loop has set up its application delegate (e.g. from the
/// eframe app creator). Logs and carries on if anything is missing.
pub fn install(wake: impl Fn() + Send + 'static) {
    *WAKE.lock_unpoisoned() = Some(Box::new(wake));
    mac::install();
}

mod mac {
    use std::sync::atomic::{AtomicPtr, Ordering};

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Imp, NSObject, Sel};
    use objc2::{define_class, msg_send, sel, ClassType};
    use objc2_foundation::NSString;

    use super::Cmd;
    use crate::sync::MutexExt;

    define_class!(
        // SAFETY: a plain NSObject subclass with no ivars and no Drop.
        #[unsafe(super(NSObject))]
        #[name = "BitChordDockTarget"]
        pub(super) struct Target;

        impl Target {
            // SAFETY: menu item actions: one argument, the sender (an NSMenuItem).
            #[unsafe(method(togglePlay:))]
            fn toggle_play(&self, _sender: &AnyObject) {
                super::push(Cmd::Toggle);
            }

            #[unsafe(method(nextTrack:))]
            fn next_track(&self, _sender: &AnyObject) {
                super::push(Cmd::Next);
            }

            #[unsafe(method(previousTrack:))]
            fn previous_track(&self, _sender: &AnyObject) {
                super::push(Cmd::Prev);
            }
        }
    );

    /// The items' target. Menu items don't retain theirs: this one is made once and never freed.
    static TARGET: AtomicPtr<Target> = AtomicPtr::new(std::ptr::null_mut());

    pub(super) fn new_target() -> Retained<Target> {
        // SAFETY: `new` on an NSObject subclass returns a retained instance.
        unsafe { msg_send![Target::class(), new] }
    }

    /// `- (NSMenu *)applicationDockMenu:(NSApplication *)sender`, added to the delegate's class.
    unsafe extern "C-unwind" fn dock_menu(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject) -> *mut AnyObject {
        let target = TARGET.load(Ordering::Relaxed);
        if target.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: `target` was leaked in `install` and lives for the whole run.
        let target: &Target = unsafe { &*target };
        let state = super::STATE.lock_unpoisoned().clone();
        // SAFETY: AppKit calls this on the main thread. Plain NSMenu/NSMenuItem calls
        // with the types they take; the menu is returned autoreleased (a getter's +0).
        unsafe {
            let menu: Retained<AnyObject> = msg_send![objc2::class!(NSMenu), new];
            if let Some(title) = &state.title {
                // No action: with the menu's auto-enabling, it shows greyed out.
                add_item(&menu, title, None, target);
                let sep: Retained<AnyObject> = msg_send![objc2::class!(NSMenuItem), separatorItem];
                let _: () = msg_send![&*menu, addItem: &*sep];
            }
            add_item(&menu, super::toggle_label(state.playing), Some(sel!(togglePlay:)), target);
            add_item(&menu, "Next", Some(sel!(nextTrack:)), target);
            add_item(&menu, "Previous", Some(sel!(previousTrack:)), target);
            Retained::autorelease_return(menu)
        }
    }

    /// Append an item to `menu`: `action` sent to `target`, or a disabled label without one.
    ///
    /// SAFETY: `menu` is an NSMenu; call on the main thread.
    unsafe fn add_item(menu: &AnyObject, title: &str, action: Option<Sel>, target: &Target) {
        // SAFETY: documented NSMenuItem setters with the types they take.
        unsafe {
            let item: Retained<AnyObject> = msg_send![objc2::class!(NSMenuItem), new];
            let _: () = msg_send![&*item, setTitle: &*NSString::from_str(title)];
            if let Some(action) = action {
                let _: () = msg_send![&*item, setAction: action];
                let _: () = msg_send![&*item, setTarget: target];
            }
            let _: () = msg_send![menu, addItem: &*item];
        }
    }

    pub(super) fn install() {
        // SAFETY: `sharedApplication` always returns the app; `delegate` returns it or nil.
        let (app, delegate) = unsafe {
            let app: Retained<AnyObject> = msg_send![objc2::class!(NSApplication), sharedApplication];
            let delegate: Option<Retained<AnyObject>> = msg_send![&*app, delegate];
            (app, delegate)
        };
        let Some(delegate) = delegate else {
            log::warn!("dock: no application delegate, so no Dock menu");
            return;
        };
        let class = delegate.class();
        let sel = sel!(applicationDockMenu:);
        if class.responds_to(sel) {
            log::warn!("dock: {} already has a Dock menu; leaving it", class.name().to_string_lossy());
            return;
        }
        let target = new_target();
        TARGET.store(Retained::as_ptr(&target).cast_mut(), Ordering::Relaxed);
        std::mem::forget(target);
        // SAFETY: the function matches the selector's signature (self, _cmd, the app;
        // returns an object), which the type encoding "@@:@" describes. Adding a
        // method to a registered class is allowed; it has none of this name yet.
        let added = unsafe {
            let imp: Imp = std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> *mut AnyObject, Imp>(dock_menu);
            objc2::ffi::class_addMethod(std::ptr::from_ref(class).cast_mut(), sel, imp, c"@@:@".as_ptr()).as_bool()
        };
        if !added {
            log::warn!("dock: couldn't add the Dock menu to {}", class.name().to_string_lossy());
            return;
        }
        // NSApplication notes which optional methods its delegate has when the delegate
        // is set: set it again so it sees the new one. (Through nil, in case setting
        // the same object is skipped.) The event loop keeps its own strong reference.
        // SAFETY: setDelegate: takes an object conforming to NSApplicationDelegate, or nil.
        unsafe {
            let none: *const AnyObject = std::ptr::null();
            let _: () = msg_send![&*app, setDelegate: none];
            let _: () = msg_send![&*app, setDelegate: &*delegate];
        }
        log::info!("dock: menu installed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    #[test]
    fn state_and_clicks() {
        set_state(Some("Song"), true);
        assert_eq!(*STATE.lock_unpoisoned(), State { title: Some("Song".into()), playing: true });
        set_state(None, false);
        assert_eq!(*STATE.lock_unpoisoned(), State::default());
        assert_eq!(toggle_label(true), "Pause");
        assert_eq!(toggle_label(false), "Play");

        // The items' actions queue commands, oldest first.
        let target = mac::new_target();
        let sender: &AnyObject = &target;
        // SAFETY: the action methods `Target` defines, with an object as the sender.
        unsafe {
            let _: () = msg_send![&*target, togglePlay: sender];
            let _: () = msg_send![&*target, nextTrack: sender];
            let _: () = msg_send![&*target, previousTrack: sender];
        }
        assert_eq!(take(), [Cmd::Toggle, Cmd::Next, Cmd::Prev]);
        assert!(take().is_empty());
    }
}
