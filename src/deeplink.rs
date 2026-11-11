//! Jam deep links: `ytm-rs://jam/CODE` and `https://grapedjuice.github.io/ytm-rs/jam/?c=CODE`.
//!
//! Clicking a link launches a new process with the URL as its argument. If another
//! instance is already running, the new one hands the code over a named pipe and
//! exits; the running instance queues it here and wakes the UI, which drains the
//! queue with `take()` (the same pattern as `tray.rs`).

use std::sync::Mutex;

use crate::jam::normalize_code as normalize;

static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// A plain later launch asked for the window (see `forward_show`).
static SHOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// What a plain later launch sends instead of a code.
const SHOW_MSG: &str = "show";
/// The updater's relaunch passes this: the old copy may still be exiting, so don't hand over to it.
pub const RELAUNCH_FLAG: &str = "--relaunch";

/// The room code in a launch argument, if it is a jam link:
/// `ytm-rs://jam/CODE[/]` (any case) or the https share link with `?c=CODE`.
pub fn parse(arg: &str) -> Option<String> {
    let arg = arg.trim().trim_matches('"');
    let (scheme, rest) = arg.split_once("://")?;
    let rest = rest.split('#').next().unwrap_or("");
    if scheme.eq_ignore_ascii_case("ytm-rs") {
        // `jam/CODE`, `jam/CODE/`, or `jam/?c=CODE`.
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let path = path.trim_end_matches('/');
        let (host, code) = path.split_once('/').unwrap_or((path, ""));
        if !host.eq_ignore_ascii_case("jam") {
            return None;
        }
        if !code.is_empty() {
            return normalize(code);
        }
        return query_param(query, "c").and_then(normalize);
    }
    if scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http") {
        let (path, query) = rest.split_once('?')?;
        let lower = path.to_ascii_lowercase();
        let path = lower.trim_end_matches('/').trim_end_matches("/index.html");
        if path != "grapedjuice.github.io/ytm-rs/jam" {
            return None;
        }
        return query_param(query, "c").and_then(normalize);
    }
    None
}

fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (k == name).then_some(v)
    })
}

/// The first jam link among the process arguments.
pub fn from_args() -> Option<String> {
    std::env::args_os().skip(1).find_map(|a| parse(&a.to_string_lossy()))
}

/// Queue a code for the UI (e.g. from the launch arguments).
pub fn push(code: String) {
    PENDING.lock().unwrap().push(code);
}

/// Codes received since the last call, oldest first.
pub fn take() -> Vec<String> {
    std::mem::take(&mut *PENDING.lock().unwrap())
}

/// Whether a later launch asked for the window since the last call.
pub fn take_show() -> bool {
    SHOW.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// A plain launch while another copy runs: bring that one's window up instead, so only
/// one copy uses (and refreshes) the Google session. Two copies refreshing it apart
/// leave one with old cookies, which Google treats as stolen and signs out. True if
/// a running copy took it.
pub fn forward_show() -> bool {
    if std::env::args_os().any(|a| a == RELAUNCH_FLAG) {
        return false;
    }
    forward_to_running(SHOW_MSG)
}

pub fn forward_to_running(_code: &str) -> bool {
    false
}

pub fn listen(_wake: impl Fn() + Send + 'static) {}

pub fn register_protocol() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_protocol() {
        assert_eq!(parse("ytm-rs://jam/ABC234").as_deref(), Some("ABC234"));
        assert_eq!(parse("ytm-rs://jam/abc234/").as_deref(), Some("ABC234"));
        assert_eq!(parse("YTM-RS://JAM/aBc234").as_deref(), Some("ABC234"));
        assert_eq!(parse("\"ytm-rs://jam/ABC234\"").as_deref(), Some("ABC234"));
        assert_eq!(parse("ytm-rs://jam/?c=xyz789").as_deref(), Some("XYZ789"));
    }

    #[test]
    fn share_link() {
        assert_eq!(parse("https://grapedjuice.github.io/ytm-rs/jam/?c=ABC234").as_deref(), Some("ABC234"));
        assert_eq!(parse("https://grapedjuice.github.io/ytm-rs/jam?c=abc234").as_deref(), Some("ABC234"));
        assert_eq!(parse("https://grapedjuice.github.io/ytm-rs/jam/index.html?x=1&c=abc234#top").as_deref(), Some("ABC234"));
    }

    #[test]
    fn rejects() {
        for bad in [
            "",
            "%1",
            "ABC234",
            "ytm-rs://jam/",
            "ytm-rs://jam/ABC23",      // too short
            "ytm-rs://jam/ABC2345",    // too long
            "ytm-rs://jam/ABC0I1",     // 0, I, 1 are not in the alphabet
            "ytm-rs://jam/ABC234/xyz", // extra path
            "ytm-rs://room/ABC234",
            "https://example.com/ytm-rs/jam/?c=ABC234",
            "https://grapedjuice.github.io/ytm-rs/?c=ABC234",
            "https://grapedjuice.github.io/ytm-rs/jam/",
            "--login",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn queue() {
        push("ABC234".into());
        push("XYZ789".into());
        assert_eq!(take(), ["ABC234", "XYZ789"]);
        assert!(take().is_empty());
    }
}
