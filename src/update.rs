//! Automatic updates from GitHub releases.
//!
//! The app checks the latest release of wenujacodes/DawnFM at startup and
//! every few hours. A copy installed from the disk image downloads the new `.dmg` and
//! puts the app from it in place of its own as it closes, or right away when the user
//! picks "Restart to update". The portable build (a `portable.txt` inside the app
//! bundle) downloads the new portable zip instead. Dev builds never update themselves.
//!
//! Release notes (the release's Markdown body) are kept per version: the one a check
//! finds, and any other on request ([`Updater::notes`]), for the update dialog, the
//! card shown after updating and Settings.
//!
//! Dev hooks: `YTM_UPDATE_FORCE=1` treats any build as installed; `YTM_UPDATE_AS=<ver>`
//! pretends to be that version; `YTM_UPDATE_DRYRUN=1` logs the install command
//! instead of running it; `YTM_UPDATE_FEED=<url>` reads the latest release from there.

use crate::sync::MutexExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const LATEST: &str = "https://api.github.com/repos/wenujacodes/DawnFM/releases/latest";
const BY_TAG: &str = "https://api.github.com/repos/wenujacodes/DawnFM/releases/tags/v";
pub const RELEASES: &str = "https://github.com/wenujacodes/DawnFM/releases";
/// The app bundle in a release download (made by `macos/bundle.sh`), and its binary.
const APP: &str = "Dawn FM.app";
const BIN_IN_APP: &str = "Contents/MacOS/dawnfm";

/// Release notes by version: `None` while fetching or when there are none.
type Notes = Arc<Mutex<std::collections::HashMap<String, Option<String>>>>;

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Downloading { version: String, percent: u8 },
    /// `installer` is the new app's binary, inside its unpacked bundle.
    Ready { version: String, installer: PathBuf },
    Failed(String),
}

#[derive(Clone)]
pub struct Updater {
    state: Arc<Mutex<State>>,
    dir: PathBuf,
    wake: Arc<dyn Fn() + Send + Sync>,
    notes: Notes,
}

pub fn current_version() -> String {
    std::env::var("YTM_UPDATE_AS").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_owned())
}

/// Whether this copy runs from an app bundle that isn't the portable one, so it's
/// Whether this copy was installed by the installer (which leaves its uninstaller
/// beside the exe), or on macOS runs from an app bundle that isn't the portable one, so it's
/// safe to update in place.
pub fn installed() -> bool {
    if std::env::var_os("YTM_UPDATE_FORCE").is_some() {
        return true;
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    if cfg!(target_os = "macos") {
        return bundle_of(&exe).is_some() && portable_dir().is_none();
    }
    let marker = if cfg!(windows) { "unins000.exe" } else { "install.txt" };
    exe.parent().is_some_and(|d| d.join(marker).exists())
}

/// The `.app` bundle a binary is in (`X.app/Contents/MacOS/dawnfm`).
fn bundle_of(exe: &Path) -> Option<&Path> {
    exe.ancestors().nth(3).filter(|a| a.extension().is_some_and(|x| x == "app"))
}

/// The release asset this copy updates from, as the release workflow names it.
fn asset_name(version: &str) -> String {
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    #[cfg(windows)]
    if portable_dir().is_some() {
        format!("DawnFM-{version}-windows-{arch}-portable.zip")
    } else {
        format!("DawnFM-{version}-windows-{arch}-setup.exe")
    }
    #[cfg(not(windows))]
    if portable_dir().is_some() {
        format!("DawnFM-{version}-macos-{arch}-portable.zip")
    } else {
        format!("DawnFM-{version}-macos-{arch}.dmg")
    }
}

/// The portable build's folder: the download ships a `portable.txt` beside the exe
/// (on macOS inside the app bundle, where code signing allows it), which keeps
/// settings in `data` next to it (next to the bundle) and has updates replace it in place.
pub fn portable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    if let Some(app) = bundle_of(&exe) {
        // Opened without being moved out of quarantine, macOS runs the app from a
        // random read-only copy; there's no folder beside it to keep data in.
        if app.to_string_lossy().contains("/AppTranslocation/") {
            return None;
        }
        if !app.join("Contents/Resources/portable.txt").exists() {
            return None;
        }
        return app.parent().map(Path::to_path_buf);
    }
    let dir = exe.parent()?.to_path_buf();
    dir.join("portable.txt").exists().then_some(dir)
}

/// Remove the exe a portable update replaced (it was still running then).
pub fn clean_portable() {
    if portable_dir().is_some()
        && let Ok(exe) = std::env::current_exe()
    {
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
    }
}

/// `a` is newer than `b` (dotted numeric versions, a leading "v" ignored).
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| v.trim_start_matches('v').split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (a, b) = (parse(a), parse(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

impl Updater {
    pub fn new(data_dir: &Path, wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self { state: Arc::new(Mutex::new(State::Idle)), dir: data_dir.join("updates"), wake: Arc::new(wake), notes: Default::default() }
    }

    pub fn state(&self) -> State {
        self.state.lock_unpoisoned().clone()
    }

    fn set(&self, s: State) {
        *self.state.lock_unpoisoned() = s;
        (self.wake)();
    }

    /// Release notes (Markdown) for `version`, fetched in the background the first time
    /// they're asked for; `None` until they arrive, or if the release has none.
    pub fn notes(&self, version: &str) -> Option<String> {
        let version = version.trim_start_matches('v').to_owned();
        {
            let mut notes = self.notes.lock_unpoisoned();
            if let Some(n) = notes.get(&version) {
                return n.clone();
            }
            notes.insert(version.clone(), None);
        }
        let this = self.clone();
        std::thread::spawn(move || {
            let fetch = async {
                let http = client()?;
                let release: serde_json::Value = http.get(format!("{BY_TAG}{version}")).send().await?.error_for_status()?.json().await?;
                anyhow::Ok(release["body"].as_str().map(str::to_owned))
            };
            let body = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt.block_on(fetch).unwrap_or_else(|e| {
                    log::warn!("update: notes for {version}: {e:#}");
                    None
                }),
                Err(_) => None,
            };
            if body.is_some() {
                this.notes.lock_unpoisoned().insert(version.clone(), body);
                (this.wake)();
            }
        });
        None
    }

    /// Check for a newer release and download it, in the background.
    pub fn check(&self) {
        if matches!(self.state(), State::Checking | State::Downloading { .. } | State::Ready { .. }) {
            return;
        }
        self.set(State::Checking);
        let this = self.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => return this.set(State::Failed(e.to_string())),
            };
            match rt.block_on(this.run()) {
                Ok(s) => this.set(s),
                Err(e) => {
                    log::warn!("update check failed: {e:#}");
                    this.set(State::Failed(format!("{e:#}")));
                }
            }
        });
    }

    async fn run(&self) -> anyhow::Result<State> {
        let http = client()?;
        let feed = std::env::var("YTM_UPDATE_FEED").unwrap_or_else(|_| LATEST.to_owned());
        let release: serde_json::Value = http.get(feed).send().await?.error_for_status()?.json().await?;
        let tag = release["tag_name"].as_str().unwrap_or_default();
        if let Some(body) = release["body"].as_str() {
            self.notes.lock_unpoisoned().insert(tag.trim_start_matches('v').to_owned(), Some(body.to_owned()));
        }
        let current = current_version();
        if !newer(tag, &current) {
            log::info!("update: {current} is current (latest {tag})");
            return Ok(State::UpToDate);
        }
        let version = tag.trim_start_matches('v').to_owned();
        let portable = portable_dir().is_some();
        let want = asset_name(&version);
        // A release published before its installer was built: nothing to install
        // yet, so this isn't an error. The next check picks it up.
        let Some(asset) = release["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"] == want.as_str())) else {
            log::info!("update: release {tag} has no {want} yet");
            return Ok(State::UpToDate);
        };
        let url = asset["browser_download_url"].as_str().ok_or_else(|| anyhow::anyhow!("asset has no URL"))?;
        let size = asset["size"].as_u64().unwrap_or(0);
        let digest = asset_digest(asset);
        if digest.is_none() {
            log::warn!("update: release {tag} gives no SHA-256 for {want}; checking its size only");
        }
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(&want);
        // A copy left from an earlier check is reused only if it still matches.
        let mut reuse = std::fs::metadata(&path).map(|m| m.len()).ok() == Some(size) && size > 0;
        if reuse && let Some(want_hash) = &digest
            && file_sha256(&path)? != *want_hash
        {
            log::warn!("update: cached {want} doesn't match its SHA-256; downloading again");
            let _ = std::fs::remove_file(&path);
            reuse = false;
        }
        if !reuse {
            log::info!("update: downloading {version} ({size} bytes)");
            self.set(State::Downloading { version: version.clone(), percent: 0 });
            let mut resp = http.get(url).send().await?.error_for_status()?;
            // Stream to a file beside, then rename, so a half-written file is never taken as ready.
            let part = path.with_extension("part");
            let mut file = std::io::BufWriter::new(std::fs::File::create(&part)?);
            let (mut got, mut shown) = (0u64, 0u8);
            // Hashed as it streams, so the file isn't read twice.
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            while let Some(chunk) = resp.chunk().await? {
                std::io::Write::write_all(&mut file, &chunk)?;
                hasher.update(&chunk);
                got += chunk.len() as u64;
                let percent = (got * 100).checked_div(size).map_or(0, |p| p.min(100) as u8);
                if percent != shown {
                    shown = percent;
                    self.set(State::Downloading { version: version.clone(), percent });
                }
            }
            std::io::Write::flush(&mut file)?;
            drop(file);
            if size > 0 && got != size {
                let _ = std::fs::remove_file(&part);
                anyhow::bail!("download incomplete ({got} of {size} bytes)");
            }
            if let Some(want_hash) = &digest {
                let got_hash = hex(&hasher.finalize());
                if got_hash != *want_hash {
                    let _ = std::fs::remove_file(&part);
                    anyhow::bail!("the download doesn't match the release's SHA-256 (expected {want_hash}, got {got_hash}); not installing it");
                }
            }
            std::fs::rename(&part, &path)?;
        }
        // Old downloads are no longer needed.
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p != path {
                    let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
                }
            }
        }
        let unpacked = self.dir.join(format!("dawnfm-{version}"));
        let ready = if !portable { open_dmg(&path, &unpacked)? } else { unzip_exe(&path, &unpacked)? };
        log::info!("update: {version} ready at {}", ready.display());
        Ok(State::Ready { version, installer: ready })
    }

    /// Run the downloaded installer silently once this process has exited, then
    /// (if `relaunch`) start the updated app.
    pub fn install(&self, relaunch: bool) {
        let State::Ready { installer, .. } = self.state() else {
            if relaunch {
                relaunch_current();
            }
            return;
        };
        let exe = std::env::current_exe().unwrap_or_default();
        #[cfg(target_os = "macos")]
        if let (Some(new), Some(app)) = (bundle_of(&installer), bundle_of(&exe)) {
            if !replace_bundle(new, app, exe.file_name(), relaunch) && relaunch {
                relaunch_current();
            }
            return;
        }
        #[cfg(unix)]
        {
            if !replace_binary(&installer, &exe, relaunch) && relaunch {
                relaunch_current();
            }
        }
        #[cfg(windows)]
        self.run_installer(&installer, &exe, relaunch);
    }

    #[cfg(windows)]
    fn run_installer(&self, installer: &Path, exe: &Path, relaunch: bool) {
        if portable_dir().is_some() {
            if !swap_exe(installer, exe, relaunch) && relaunch {
                relaunch_windows_exe(exe);
            }
            return;
        }
        let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
        let scope = if program_files.is_some_and(|p| exe.starts_with(p)) { "/ALLUSERS" } else { "/CURRENTUSER" };
        let mut script = format!(
            "ping -n 4 127.0.0.1 >NUL & \"{}\" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART {scope}",
            installer.display()
        );
        if relaunch {
            script += &format!(" & start \"\" \"{}\" {}", exe.display(), crate::deeplink::RELAUNCH_FLAG);
        }
        if std::env::var_os("YTM_UPDATE_DRYRUN").is_some() {
            log::info!("update (dry run): cmd /C {script}");
            return;
        }
        log::info!("update: installing {}", installer.display());
        run_detached(&script);
    }
}

/// Start this app again once the current process has finished exiting.
pub fn relaunch_current() {
    let Ok(exe) = std::env::current_exe() else { return };
    log::info!("restarting {}", exe.display());
    #[cfg(target_os = "macos")]
    if let Some(app) = bundle_of(&exe) {
        relaunch_bundle(app, exe.file_name());
        return;
    }
    #[cfg(unix)]
    {
        relaunch_unix_exe(&exe);
    }
    #[cfg(windows)]
    {
        relaunch_windows_exe(&exe);
    }
}

/// The portable update: Windows lets a running exe be renamed, so move this one aside
/// and put the new one in its place (the next launch deletes the old one).
#[cfg(windows)]
fn swap_exe(new: &Path, exe: &Path, relaunch: bool) -> bool {
    let old = exe.with_extension("exe.old");
    if std::env::var_os("YTM_UPDATE_DRYRUN").is_some() {
        log::info!("update (dry run): {} aside to {}, {} in its place", exe.display(), old.display(), new.display());
        return true;
    }
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(exe, &old) {
        log::warn!("update: couldn't move {} aside: {e}", exe.display());
        return false;
    }
    if let Err(e) = std::fs::copy(new, exe) {
        log::warn!("update: couldn't copy the new version: {e}");
        let _ = std::fs::rename(&old, exe);
        return false;
    }
    log::info!("update: replaced {}", exe.display());
    if relaunch {
        relaunch_windows_exe(exe);
    }
    true
}

#[cfg(windows)]
fn relaunch_windows_exe(exe: &Path) {
    run_detached(&format!("ping -n 4 127.0.0.1 >NUL & start \"\" \"{}\" {}", exe.display(), crate::deeplink::RELAUNCH_FLAG));
}

/// A bare binary (not in a bundle): rename the new one over this one. The running
/// process keeps its copy open until it exits; the next launch runs the new one.
#[cfg(unix)]
fn replace_binary(new: &Path, exe: &Path, relaunch: bool) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let tmp = exe.with_file_name(".dawnfm.new");
    if std::env::var_os("YTM_UPDATE_DRYRUN").is_some() {
        log::info!("update (dry run): {} over {}", new.display(), exe.display());
        return true;
    }
    let done = std::fs::copy(new, &tmp)
        .and_then(|_| std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)))
        .and_then(|_| std::fs::rename(&tmp, exe));
    if let Err(e) = done {
        let _ = std::fs::remove_file(&tmp);
        log::warn!("update: couldn't replace {}: {e}", exe.display());
        return false;
    }
    log::info!("update: replaced {}", exe.display());
    if relaunch {
        relaunch_unix_exe(exe);
    }
    true
}

/// macOS: put the new app bundle where this one is, so its Info.plist and icon update
/// too. A running app's bundle can be moved; this process keeps running from the old
/// one until it exits.
#[cfg(target_os = "macos")]
fn replace_bundle(new: &Path, app: &Path, bin_name: Option<&std::ffi::OsStr>, relaunch: bool) -> bool {
    let parent = app.parent().unwrap_or(Path::new("/"));
    let (tmp, old) = (parent.join(".dawnfm.app.new"), parent.join(".dawnfm.app.old"));
    // A copy comes back as Dawn FM.app.
    let renamed = parent.join(APP);
    let dest = if app.file_name() != renamed.file_name() && !renamed.exists() { renamed } else { app.to_path_buf() };
    if std::env::var_os("YTM_UPDATE_DRYRUN").is_some() {
        log::info!("update (dry run): {} in place of {}", new.display(), app.display());
        return true;
    }
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(&old);
    // ditto copies a bundle whole: symlinks, permissions, signature.
    let copied = std::process::Command::new("ditto").arg(new).arg(&tmp).status().is_ok_and(|s| s.success());
    if !copied {
        let _ = std::fs::remove_dir_all(&tmp);
        log::warn!("update: couldn't copy {} beside {}", new.display(), app.display());
        return false;
    }
    let _ = std::process::Command::new("xattr").args(["-cr"]).arg(&tmp).status();
    if let Err(e) = std::fs::rename(app, &old) {
        let _ = std::fs::remove_dir_all(&tmp);
        log::warn!("update: couldn't move {} aside: {e}", app.display());
        return false;
    }
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::rename(&old, app);
        let _ = std::fs::remove_dir_all(&tmp);
        log::warn!("update: couldn't put the new version in place: {e}");
        return false;
    }
    let _ = std::fs::remove_dir_all(&old);
    log::info!("update: replaced {} with {}", app.display(), dest.display());
    if relaunch {
        relaunch_bundle(&dest, bin_name);
    }
    true
}

#[cfg(unix)]
fn shell_quote(path: &Path) -> String {
    path.display().to_string().replace('\'', r"'\''")
}

#[cfg(unix)]
fn relaunch_unix_exe(exe: &Path) {
    let pid = std::process::id();
    let quoted = shell_quote(exe);
    run_detached(&format!(
        "i=0; while kill -0 {pid} 2>/dev/null && [ $i -lt 200 ]; do sleep 0.1; i=$((i+1)); done; \
         if kill -0 {pid} 2>/dev/null; then kill -TERM {pid} 2>/dev/null || true; sleep 0.5; fi; \
         sleep 0.3; \
         exec '{quoted}' {}",
        crate::deeplink::RELAUNCH_FLAG,
    ));
}

#[cfg(target_os = "macos")]
fn bundle_relaunch_script(app: &Path, bin_name: Option<&std::ffi::OsStr>, pid: u32) -> String {
    let bundle = shell_quote(app);
    let leaf = bin_name.filter(|s| !s.is_empty()).unwrap_or_else(|| std::ffi::OsStr::new("dawnfm"));
    let bin = app.join("Contents/MacOS").join(leaf);
    let bin = shell_quote(&bin);
    let proc_name = shell_quote(Path::new(leaf));
    let flag = crate::deeplink::RELAUNCH_FLAG;
    // Wait until the exiting process is completely gone before calling LaunchServices:
    // `open` without `-n` activates the dying instance instead of launching the new one,
    // and launching too early can race with persistence writes or LaunchServices teardown
    // of a freshly replaced bundle. Retry while verifying a new process actually started.
    format!(
        "i=0; while kill -0 {pid} 2>/dev/null && [ $i -lt 200 ]; do sleep 0.1; i=$((i+1)); done; \
         if kill -0 {pid} 2>/dev/null; then kill -TERM {pid} 2>/dev/null || true; sleep 0.5; fi; \
         sleep 0.5; \
         xattr -cr '{bundle}' 2>/dev/null || xattr -dr com.apple.quarantine '{bundle}' 2>/dev/null || true; \
         /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f '{bundle}' >/dev/null 2>&1 || true; \
         launched=0; j=0; \
         while [ $j -lt 5 ]; do \
             if open -n '{bundle}' --args {flag} >/dev/null 2>&1; then \
                 sleep 0.6; \
                 if pgrep -x '{proc_name}' 2>/dev/null | grep -vq '^{pid}$'; then launched=1; break; fi; \
             fi; \
             sleep 0.5; \
             j=$((j+1)); \
         done; \
         [ $launched -eq 1 ] || open '{bundle}' --args {flag} >/dev/null 2>&1 || exec '{bin}' {flag} >/dev/null 2>&1"
    )
}

#[cfg(target_os = "macos")]
fn relaunch_bundle(app: &Path, bin_name: Option<&std::ffi::OsStr>) {
    let script = bundle_relaunch_script(app, bin_name, std::process::id());
    run_detached(&script);
}

/// macOS: the app from a downloaded installer disk image, copied out into `dir`.
fn open_dmg(dmg: &Path, dir: &Path) -> anyhow::Result<PathBuf> {
    let _ = std::fs::remove_dir_all(dir);
    let mnt = dir.join("mnt");
    std::fs::create_dir_all(&mnt)?;
    let attached = std::process::Command::new("hdiutil").args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-quiet", "-mountpoint"]).arg(&mnt).arg(dmg).status()?;
    anyhow::ensure!(attached.success(), "couldn't open {}", dmg.display());
    let copied = std::process::Command::new("ditto").arg(mnt.join(APP)).arg(dir.join(APP)).status();
    let _ = std::process::Command::new("hdiutil").args(["detach", "-quiet"]).arg(&mnt).status();
    let exe = dir.join(APP).join(BIN_IN_APP);
    anyhow::ensure!(copied.is_ok_and(|s| s.success()) && exe.exists(), "couldn't copy the app out of {}", dmg.display());
    Ok(exe)
}

/// The new binary from a downloaded portable zip, unpacked into `dir` with tar.
fn unzip_exe(zip: &Path, dir: &Path) -> anyhow::Result<PathBuf> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir)?;
    #[cfg(windows)]
    let mut cmd = {
        let system = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        std::process::Command::new(system.join(r"System32\tar.exe"))
    };
    #[cfg(not(windows))]
    let mut cmd = std::process::Command::new("tar");
    cmd.arg("-xf").arg(zip).arg("-C").arg(dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let status = cmd.status()?;
    #[cfg(windows)]
    let exe = dir.join("dawnfm.exe");
    // Everything is in one top-level folder: the binary, or on macOS the app bundle.
    #[cfg(not(windows))]
    let exe = std::iter::once(dir.join(APP).join(BIN_IN_APP))
        .chain(std::fs::read_dir(dir)?.flatten().map(|e| e.path().join(APP).join(BIN_IN_APP)))
        .find(|p| p.is_file())
        .unwrap_or_else(|| dir.join(APP).join(BIN_IN_APP));
    anyhow::ensure!(status.success() && exe.exists(), "couldn't unpack {}", zip.display());
    Ok(exe)
}

/// Run a command/script detached and windowless, so it outlives this process.
fn run_detached(script: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        if let Err(e) = std::process::Command::new("cmd").arg("/C").raw_arg(script).creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS).spawn() {
            log::warn!("update: couldn't run the update: {e}");
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        use std::process::Stdio;
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // New session & process group so closing the parent never terminates the relaunch helper.
        // SAFETY: `setsid` / `setpgid` are async-signal-safe between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    let _ = libc::setpgid(0, 0);
                }
                Ok(())
            });
        }
        if let Err(e) = cmd.spawn() {
            log::warn!("update: couldn't run the update: {e}");
        }
    }
}

/// The SHA-256 GitHub gives for a release asset (`"digest": "sha256:<hex>"`), as
/// lowercase hex; `None` for older releases without one.
fn asset_digest(asset: &serde_json::Value) -> Option<String> {
    let hex = asset["digest"].as_str()?.strip_prefix("sha256:")?.to_ascii_lowercase();
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hex)
}

/// A file's SHA-256 as lowercase hex, read in pieces.
fn file_sha256(path: &Path) -> anyhow::Result<String> {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(crate::proxy::apply(reqwest::Client::builder()).user_agent(concat!("DawnFM/", env!("CARGO_PKG_VERSION"))).connect_timeout(std::time::Duration::from_secs(10)).read_timeout(std::time::Duration::from_secs(30)).build()?)
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn asset_digests() {
        let a = |d: serde_json::Value| super::asset_digest(&serde_json::json!({ "name": "x.dmg", "digest": d }));
        let hex = "FFD9938D1CEC7E3C4A3F1B3078D2FB9650D9AE88137AA0C3CE19D4176F73C02F";
        assert_eq!(a(format!("sha256:{hex}").into()), Some(hex.to_ascii_lowercase()));
        assert_eq!(a(serde_json::Value::Null), None);
        assert_eq!(a("sha1:abcd".into()), None);
        assert_eq!(a("sha256:abcd".into()), None);
        assert_eq!(a(format!("sha256:{}", "zz".repeat(32)).into()), None);
        // The streamed hash and the file hash agree with a known value.
        let path = std::env::temp_dir().join(format!("bitchord-digest-test-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(super::file_sha256(&path).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn version_order() {
        assert!(newer("v0.2.8", "0.2.7"));
        assert!(newer("v0.10.0", "0.9.9"));
        assert!(newer("1.0", "0.99.99"));
        assert!(!newer("v0.2.7", "0.2.7"));
        assert!(!newer("v0.2.6", "0.2.7"));
        assert!(!newer("", "0.2.7"));
    }

    /// A fake bundle packed the way the release workflow packs it (`ditto` zip and
    /// `hdiutil` disk image), unpacked the way an update does.
    #[test]
    #[cfg(target_os = "macos")]
    fn finds_the_app_in_release_downloads() {
        let root = std::env::temp_dir().join(format!("bitchord-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let src = root.join("src");
        let bin = src.join(super::APP).join(super::BIN_IN_APP);
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"#!/bin/sh\n").unwrap();
        let run = |cmd: &mut std::process::Command| assert!(cmd.status().unwrap().success());

        let zip = root.join("app.zip");
        run(std::process::Command::new("ditto").args(["-c", "-k", "--keepParent"]).arg(src.join(super::APP)).arg(&zip));
        let exe = super::unzip_exe(&zip, &root.join("from-zip")).unwrap();
        assert!(exe.ends_with(format!("{}/{}", super::APP, super::BIN_IN_APP)), "{}", exe.display());

        // CI runners can refuse to mount disk images; the zip half above still runs there.
        if std::env::var_os("CI").is_some() {
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        let dmg = root.join("app.dmg");
        run(std::process::Command::new("hdiutil").args(["create", "-quiet", "-volname", "Dawn FM", "-format", "UDZO", "-srcfolder"]).arg(&src).arg(&dmg));
        let exe = super::open_dmg(&dmg, &root.join("from-dmg")).unwrap();
        assert!(exe.is_file(), "{}", exe.display());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn bundle_relaunch_waits_for_pid_uses_new_instance_and_falls_back_to_binary() {
        let s = super::bundle_relaunch_script(std::path::Path::new("/Applications/Dawn FM.app"), None, 4242);
        assert!(s.contains("kill -0 4242"));
        assert!(s.contains("lsregister -f '/Applications/Dawn FM.app'"));
        assert!(s.contains("pgrep -x 'dawnfm'"));
        assert!(s.contains("open -n '/Applications/Dawn FM.app' --args --relaunch"));
        assert!(s.contains("exec '/Applications/Dawn FM.app/Contents/MacOS/dawnfm' --relaunch"));
        assert!(std::process::Command::new("sh").args(["-n", "-c", &s]).status().unwrap().success());
    }
}
