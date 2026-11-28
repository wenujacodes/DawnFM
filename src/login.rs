//! "Sign in with Google" and "Sign in to Spotify" windows.
//!
//! Opens the service's normal sign-in page in the system webview (WebView2 on Windows,
//! WKWebView on macOS, WebKitGTK on Linux), waits until the flow lands on the app's
//! site, then hands the session cookies (HttpOnly ones included) to the app.
//!
//! The window runs in a child process (`ytm-rs --login <dir>`): tao and winit crash
//! when both pump windows in one process (STATUS_FATAL_USER_CALLBACK_EXCEPTION), and a
//! separate process also returns all webview memory to the OS once sign-in ends.

use std::path::PathBuf;

use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::WindowBuilder;
use wry::{WebContext, WebViewBuilder};

/// Which sign-in.
#[derive(Clone, Copy)]
pub enum Site {
    /// youtube.com, for YouTube Music.
    Google,
    /// open.spotify.com, for the Spotify transfer.
    Spotify,
}

impl Site {
    fn flag(self) -> &'static str {
        match self {
            Site::Google => FLAG,
            Site::Spotify => SPOTIFY_FLAG,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Site::Google => "Sign in to YouTube Music",
            Site::Spotify => "Sign in to Spotify",
        }
    }

    fn start(self) -> &'static str {
        match self {
            Site::Google => "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F",
            Site::Spotify => "https://accounts.spotify.com/en/login?continue=https%3A%2F%2Fopen.spotify.com%2F",
        }
    }

    /// Where the flow lands once signed in.
    fn home(self) -> &'static str {
        match self {
            Site::Google => "https://music.youtube.com/",
            Site::Spotify => "https://open.spotify.com/",
        }
    }

    /// The cookie that only a signed-in session has.
    fn session_cookie(self) -> &'static str {
        match self {
            Site::Google => "SAPISID",
            Site::Spotify => "sp_dc",
        }
    }
}

pub enum Outcome {
    /// Google: the `Cookie` header value for youtube.com. Spotify: the `sp_dc` cookie's value.
    Cookie(String),
    Cancelled,
    Failed(String),
}

/// Safari's user agent (macOS always reports 10_15_7 here, whatever the version).
const SAFARI_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

/// Command-line flags that make the binary run only a sign-in window.
pub const FLAG: &str = "--login";
pub const SPOTIFY_FLAG: &str = "--login-spotify";

/// Show the sign-in window in a child process; `done` is called exactly once.
pub fn spawn(site: Site, profile_dir: PathBuf, done: impl FnOnce(Outcome) + Send + 'static) {
    std::thread::Builder::new()
        .name("login".into())
        .spawn(move || {
            let res = std::env::current_exe().and_then(|exe| {
                let mut cmd = std::process::Command::new(exe);
                crate::proxy::pass_to(&mut cmd);
                cmd.arg(site.flag())
                    .arg(&profile_dir)
                    .stdin(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .output()
            });
            done(match res {
                Err(e) => Outcome::Failed(format!("could not start sign-in window: {e}")),
                Ok(out) => {
                    let text = String::from_utf8_lossy(&out.stdout);
                    let line = text.lines().last().unwrap_or_default();
                    if let Some(c) = line.strip_prefix("cookie ") {
                        Outcome::Cookie(c.to_owned())
                    } else if let Some(e) = line.strip_prefix("failed ") {
                        Outcome::Failed(e.to_owned())
                    } else if line == "cancelled" {
                        Outcome::Cancelled
                    } else {
                        Outcome::Failed(format!("sign-in window exited unexpectedly ({})", out.status))
                    }
                }
            })
        })
        .expect("spawn login thread");
}

/// Entry point of the child process: run the window and report on stdout.
pub fn child_main(site: Site, profile_dir: PathBuf) {
    match run(site, profile_dir) {
        Ok(Outcome::Cookie(c)) => println!("cookie {c}"),
        Ok(Outcome::Cancelled) => println!("cancelled"),
        Ok(Outcome::Failed(e)) => println!("failed {e}"),
        Err(e) => println!("failed {e:#}"),
    }
}

/// Put the webview in its window.
pub trait BuildIn<'a> {
    fn build_in(self, window: &'a tao::window::Window) -> wry::Result<wry::WebView>;
}

impl<'a> BuildIn<'a> for WebViewBuilder<'a> {
    fn build_in(self, window: &'a tao::window::Window) -> wry::Result<wry::WebView> {
        self.build(window)
    }
}

/// The cookies the browser would send to `url`.
#[cfg(not(target_os = "macos"))]
fn cookies_for(webview: &wry::WebView, url: &str) -> wry::Result<Vec<wry::cookie::Cookie<'static>>> {
    webview.cookies_for_url(url)
}

/// On macOS wry's `cookies_for_url` keeps only cookies set for exactly the page's host, so
/// Google's, set for `.youtube.com`, never came back and signing in never finished.
#[cfg(target_os = "macos")]
fn cookies_for(webview: &wry::WebView, url: &str) -> wry::Result<Vec<wry::cookie::Cookie<'static>>> {
    let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
    Ok(webview.cookies()?.into_iter().filter(|c| c.domain().is_some_and(|d| domain_matches(&host, d))).collect())
}

/// Whether a cookie for `domain` (".youtube.com", "youtube.com", "music.youtube.com")
/// goes to `host`.
fn domain_matches(host: &str, domain: &str) -> bool {
    let domain = domain.trim_start_matches('.').to_ascii_lowercase();
    let host = host.to_ascii_lowercase();
    !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}")))
}

/// Delete the webview profile so the next sign-in starts fresh (on sign-out, and after
/// signing in). The browser processes can hold its files for a moment after the
/// window closes, so it tries again for a few seconds; blocks meanwhile.
pub fn forget(profile_dir: &std::path::Path) {
    for tries in 1..=10 {
        match std::fs::remove_dir_all(profile_dir) {
            Ok(()) => return,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) if tries == 10 => log::warn!("couldn't delete the sign-in window's profile {}: {e}", profile_dir.display()),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
        }
    }
}

/// WKWebView has no profile folder (wry ignores the context's on macOS), so each
/// profile gets a data store of its own (macOS 14+; older ones share the default),
/// named by an id kept in the folder. `forget` deletes the folder, so the next sign-in
/// starts from a fresh store.
#[cfg(target_os = "macos")]
fn store_id(profile_dir: &std::path::Path) -> [u8; 16] {
    use sha1::Digest;
    let file = profile_dir.join("store-id");
    if let Ok(saved) = std::fs::read(&file)
        && let Ok(id) = <[u8; 16]>::try_from(saved.as_slice())
    {
        return id;
    }
    let seed = format!("{} {:?} {}", profile_dir.display(), std::time::SystemTime::now(), std::process::id());
    let mut id = [0u8; 16];
    id.copy_from_slice(&sha1::Sha1::digest(seed)[..16]);
    let _ = std::fs::create_dir_all(profile_dir);
    let _ = std::fs::write(&file, id);
    id
}

fn run(site: Site, profile_dir: PathBuf) -> anyhow::Result<Outcome> {
    let mut event_loop = EventLoopBuilder::<()>::with_user_event().build();
    #[cfg(target_os = "macos")]
    configure_macos_accessory(&mut event_loop);

    let window = WindowBuilder::new()
        .with_title(site.title())
        .with_inner_size(tao::dpi::LogicalSize::new(480.0, 720.0))
        .build(&event_loop)?;

    if let Some(monitor) = window.primary_monitor().or_else(|| window.current_monitor()) {
        let screen_size = monitor.size();
        let scale = monitor.scale_factor();
        let win_w = (480.0 * scale) as i32;
        let win_h = (720.0 * scale) as i32;
        let x = (screen_size.width as i32 - win_w) / 2;
        let y = (screen_size.height as i32 - win_h) / 2;
        window.set_outer_position(tao::dpi::PhysicalPosition::new(x.max(0), y.max(0)));
    }

    let proxy = event_loop.create_proxy();
    let proxy_nav = event_loop.create_proxy();
    let proxy_retry = event_loop.create_proxy();
    #[cfg(target_os = "macos")]
    let store = store_id(&profile_dir);
    let mut context = WebContext::new(Some(profile_dir));
    let builder = WebViewBuilder::new_with_web_context(&mut context);
    #[cfg(target_os = "macos")]
    let builder = {
        use wry::WebViewBuilderExtDarwin;
        // WKWebView's own user agent has no "Version/… Safari/…", so YouTube Music takes it
        // for an unknown browser and shows "not optimized for your browser" instead of the
        // site. Say it's Safari, which is the engine it is.
        builder.with_data_store_identifier(store).with_user_agent(SAFARI_UA)
    };
    // Google binds a sign-in from a browser that offers device-bound sessions to a key
    // only that browser holds (`__Secure-1PSIDRTS`, renewed every 10 minutes), so the
    // copy the app takes stops working after some hours. Without them Google signs in
    // the way it does Firefox and Safari, and the copy lasts.
    let webview = crate::proxy::webview(builder, matches!(site, Site::Google))
        .with_url(site.start())
        .with_navigation_handler(move |url| {
            if url.starts_with(site.home()) {
                let _ = proxy_nav.send_event(());
            }
            true
        })
        .with_on_page_load_handler(move |_ev, url| {
            if url.starts_with(site.home()) {
                let _ = proxy.send_event(());
            }
        })
        .build_in(&window)?;

    let mut outcome = Outcome::Cancelled;
    event_loop.run_return(|event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(()) => {
                // Immediately hide the window once redirected to the home site so
                // the user never sees the web interface loading inside the dialog.
                window.set_visible(false);
                match cookies_for(&webview, site.home()) {
                    Ok(cookies) => {
                        // Landing there while signed out also fires this; only a signed-in
                        // session has the session cookie (SAPISID, sp_dc).
                        if let Some(c) = cookies.iter().find(|c| c.name() == site.session_cookie()) {
                            outcome = Outcome::Cookie(match site {
                                Site::Spotify => c.value().to_owned(),
                                Site::Google => cookies.iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; "),
                            });
                            *flow = ControlFlow::Exit;
                        } else {
                            // If cookies are still synchronizing from the redirect, retry in a brief moment
                            let retry = proxy_retry.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(std::time::Duration::from_millis(150));
                                let _ = retry.send_event(());
                            });
                        }
                    }
                    Err(e) => {
                        outcome = Outcome::Failed(format!("could not read cookies: {e}"));
                        *flow = ControlFlow::Exit;
                    }
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
    drop(webview);
    Ok(outcome)
}

/// Keep the child sign-in window out of the macOS Dock while still allowing it to
/// activate and accept keyboard input in front of the main Dawn FM window.
#[cfg(target_os = "macos")]
fn configure_macos_accessory(event_loop: &mut tao::event_loop::EventLoop<()>) {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};

    const NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY: isize = 1;
    unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![objc2::class!(NSApplication), sharedApplication];
        if let Some(app) = app {
            let _: bool = msg_send![&*app, setActivationPolicy: NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY];
        }
    }
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    event_loop.set_dock_visibility(false);
    event_loop.set_activate_ignoring_other_apps(true);
}

#[cfg(test)]
mod tests {
    use super::domain_matches;

    #[test]
    fn parent_domain_cookies_go_to_subdomains() {
        assert!(domain_matches("music.youtube.com", ".youtube.com"));
        assert!(domain_matches("music.youtube.com", "youtube.com"));
        assert!(domain_matches("music.youtube.com", "music.youtube.com"));
        assert!(!domain_matches("music.youtube.com", "accounts.google.com"));
        assert!(!domain_matches("music.youtube.com", "tube.com"));
        assert!(!domain_matches("evilyoutube.com", ".youtube.com"));
    }
}
