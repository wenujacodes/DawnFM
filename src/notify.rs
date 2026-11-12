//! "Notify on song change": a macOS notification with the new song.
//!
//! Through `osascript`'s `display notification`, which works for any process: no
//! bundle, signing, or authorization request, unlike UNUserNotificationCenter (which
//! needs a signed bundle and a permission prompt) or the deprecated
//! NSUserNotificationCenter (nil outside a bundle, and quietly dropped on some recent
//! macOS versions). The text goes in as script arguments, never into the script
//! itself, so nothing in a title needs escaping. Failures are logged once.

use std::sync::atomic::{AtomicBool, Ordering};

/// Reads the title and the message from the arguments after the script.
const SCRIPT: [&str; 3] = ["on run argv", "display notification (item 2 of argv) with title (item 1 of argv)", "end run"];

/// The osascript arguments that show `body` under `title`.
fn args(title: &str, body: &str) -> Vec<String> {
    let mut a = Vec::new();
    for line in SCRIPT {
        a.push("-e".to_owned());
        a.push(line.to_owned());
    }
    // `--`: a title starting with "-" is an argument, not an option.
    a.extend(["--".to_owned(), title.to_owned(), body.to_owned()]);
    a
}

/// What the notification says: "Title — Artist" (or just the title).
pub fn text(title: &str, artist: &str) -> String {
    if artist.is_empty() { title.to_owned() } else { format!("{title} — {artist}") }
}

/// Show `body` as a notification from BitChord, without waiting for it.
pub fn post(body: String) {
    static WARNED: AtomicBool = AtomicBool::new(false);
    std::thread::spawn(move || {
        let ok = match std::process::Command::new("/usr/bin/osascript").args(args("Dusk FM", &body)).output() {
            Ok(out) if out.status.success() => return,
            Ok(out) => format!("{}: {}", out.status, String::from_utf8_lossy(&out.stderr).trim()),
            Err(e) => e.to_string(),
        };
        if !WARNED.swap(true, Ordering::Relaxed) {
            log::warn!("notify: couldn't show a notification ({ok})");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_args() {
        assert_eq!(text("Song", "Artist"), "Song — Artist");
        assert_eq!(text("Song", ""), "Song");
        // The text is passed as-is, after `--`: quotes and backslashes can't break out.
        let body = text("\"; do shell script \"rm -rf ~", "-a \\ b");
        let a = args("Dusk FM", &body);
        assert_eq!(a.len(), 9);
        assert_eq!(&a[6..], ["--", "Dusk FM", body.as_str()]);
        assert!(a[..6].iter().all(|s| !s.contains("rm")));
    }
}
