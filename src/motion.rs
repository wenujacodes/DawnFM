//! Apple's motion sleeves, played locally.
//!
//! The clip is HLS, but each variant's media playlist is a single byte-ranged
//! fragmented MP4, so it downloads as one file and Media Foundation decodes it
//! off the UI thread. Only the newest frame is kept: the UI turns it into a
//! texture and draws it in place of the still sleeve.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::Duration;

pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

pub struct Motion {
    url: String,
    latest: Mutex<Option<Arc<Frame>>>,
    version: AtomicU64,
    stop: Arc<AtomicBool>,
}

impl Motion {
    fn new(url: &str) -> Self {
        Self { url: url.to_owned(), latest: Mutex::new(None), version: AtomicU64::new(0), stop: Arc::new(AtomicBool::new(false)) }
    }

    /// The newest frame, with the counter that says it changed.
    pub fn latest(&self) -> Option<(u64, Arc<Frame>)> {
        let frame = self.latest.lock().ok()?.clone()?;
        Some((self.version.load(Ordering::Acquire), frame))
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// Stop decoding: the page that showed this clip is gone.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn publish(&self, ctx: &egui::Context, w: usize, h: usize, rgba: Vec<u8>) {
        if let Ok(mut slot) = self.latest.lock() {
            *slot = Some(Arc::new(Frame { w, h, rgba }));
        }
        self.version.fetch_add(1, Ordering::Release);
        ctx.request_repaint();
    }
}

impl Drop for Motion {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

static STREAMS: LazyLock<Mutex<HashMap<String, Weak<Motion>>>> = LazyLock::new(Default::default);

/// Start (or reuse) the clip at `master`, decoded no wider than `max_width`.
pub fn play(ctx: &egui::Context, master: &str, max_width: u32) -> Arc<Motion> {
    let key = format!("{max_width}:{master}");
    if let Ok(mut streams) = STREAMS.lock() {
        if let Some(m) = streams.get(&key).and_then(Weak::upgrade) {
            return m;
        }
        let motion = Arc::new(Motion::new(master));
        streams.insert(key, Arc::downgrade(&motion));
        let (this, ctx, url) = (motion.clone(), ctx.clone(), master.to_owned());
        let _ = std::thread::Builder::new().name("motion".into()).spawn(move || run(this, ctx, url, max_width));
        return motion;
    }
    Arc::new(Motion::new(master))
}

fn run(motion: Arc<Motion>, ctx: egui::Context, master: String, max_width: u32) {
    let Some(path) = fetch(&master, max_width) else {
        log::info!("apple: no motion clip downloaded");
        return;
    };
    decode(&motion, &path, &ctx);
}

/// Pick a variant and save the clip to the cache; a saved copy is reused.
fn fetch(master: &str, max_width: u32) -> Option<PathBuf> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
    rt.block_on(async {
        use sha1::{Digest, Sha1};
        let http = crate::proxy::http();
        let media = crate::apple::variant(&http, master, max_width).await?;
        let file = crate::apple::media_file(&http, &media).await?;
        let dir = crate::data_dir().join("motion");
        std::fs::create_dir_all(&dir).ok()?;
        let name = format!("{:x}", Sha1::digest(file.as_bytes()));
        let path = dir.join(format!("{}.mp4", &name[..32]));
        if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > 1024 {
            return Some(path);
        }
        let bytes = http
            .get(&file)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .bytes()
            .await
            .ok()?;
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, &bytes).ok()?;
        std::fs::rename(&tmp, &path).ok()?;
        log::info!("apple: motion clip {:.0} KB cached", bytes.len() as f64 / 1024.0);
        Some(path)
    })
}

/// Decoding the motion clips is not available on macOS.
fn decode(_motion: &Arc<Motion>, _path: &std::path::Path, _ctx: &egui::Context) {}
