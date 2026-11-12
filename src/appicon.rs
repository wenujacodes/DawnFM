//! The app icon for the window, the tray and the Start menu, desktop and pinned
//! taskbar shortcuts that launch this exe. The exe's own embedded icon (Explorer's
//! file view) stays the classic one.
//!
//! The choice lives in `<data dir>/app_icon` as plain text, so `main` can read it
//! before the window opens.

use std::path::Path;

pub struct Variant {
    pub id: &'static str,
    #[allow(dead_code)] // display name, for when the icon picker comes back
    pub name: &'static str,
    /// An ICO whose entries are PNGs, from 16 to 256 px.
    ico: &'static [u8],
}

macro_rules! variant {
    ($id:literal, $name:literal) => {
        Variant { id: $id, name: $name, ico: include_bytes!(concat!("../assets/icon/variants/", $id, ".ico")) }
    };
}

pub const CLASSIC: &str = "classic";

pub static ALL: &[Variant] = &[
    Variant { id: CLASSIC, name: "BitChord", ico: include_bytes!("../assets/icon/icon.ico") },
    variant!("official", "Original"),
    variant!("smilering", "Smile ring"),
    variant!("gear", "Rust gear"),
    variant!("wave", "Waveform"),
    variant!("vinyl", "Record"),
    variant!("lyrics", "Lit line"),
    variant!("duet", "Duet"),
    variant!("kawarp", "Warp"),
    variant!("fluent", "Fluent"),
];

pub fn get(id: &str) -> &'static Variant {
    ALL.iter().find(|v| v.id == id).unwrap_or(&ALL[0])
}

impl Variant {
    /// The PNG of the smallest entry at least `size` px wide (else the largest).
    pub fn png(&self, size: u32) -> &'static [u8] {
        let entries = ico_entries(self.ico);
        entries
            .iter()
            .filter(|(w, _)| *w >= size)
            .min_by_key(|(w, _)| *w)
            .or_else(|| entries.iter().max_by_key(|(w, _)| *w))
            .map_or(&[][..], |(_, png)| *png)
    }

    /// Pixels at `size` px, for the window and tray icons.
    pub fn rgba(&self, size: u32) -> Option<image::RgbaImage> {
        let img = image::load_from_memory(self.png(size)).ok()?.into_rgba8();
        Some(if img.width() == size { img } else { image::imageops::resize(&img, size, size, image::imageops::FilterType::Lanczos3) })
    }

    pub fn icon_data(&self) -> Option<egui::IconData> {
        let img = self.rgba(256)?;
        Some(egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() })
    }
}

/// (width, PNG bytes) of each entry. A width byte of 0 means 256.
fn ico_entries(ico: &'static [u8]) -> Vec<(u32, &'static [u8])> {
    let u16_at = |i: usize| ico.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32_at = |i: usize| ico.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let n = u16_at(4).unwrap_or(0);
    (0..n)
        .filter_map(|i| {
            let e = 6 + 16 * i;
            let w = match *ico.get(e)? {
                0 => 256,
                w => w as u32,
            };
            let (len, off) = (u32_at(e + 8)?, u32_at(e + 12)?);
            Some((w, ico.get(off..off + len)?))
        })
        .collect()
}

pub fn load(data_dir: &Path) -> &'static str {
    let id = std::fs::read_to_string(data_dir.join("app_icon")).unwrap_or_default();
    get(id.trim()).id
}

/// Point shortcuts at the chosen icon. macOS has no shortcuts to update.
pub fn sync_shortcuts(_id: &'static str, _data_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_has_every_size() {
        for v in ALL {
            let sizes: Vec<u32> = ico_entries(v.ico).iter().map(|(w, _)| *w).collect();
            assert!(sizes.contains(&16) && sizes.contains(&32) && sizes.contains(&256), "{}: {sizes:?}", v.id);
            for size in [16, 24, 32, 56, 256] {
                let img = v.rgba(size).unwrap_or_else(|| panic!("{} doesn't decode at {size}", v.id));
                assert_eq!(img.dimensions(), (size, size), "{}", v.id);
            }
        }
    }

    #[test]
    fn picks_the_smallest_entry_that_fits() {
        let v = get("gear");
        let w = |png: &[u8]| image::load_from_memory(png).unwrap().width();
        assert_eq!(w(v.png(16)), 16);
        assert_eq!(w(v.png(50)), 64);
        assert_eq!(w(v.png(300)), 256);
        assert_eq!(get("nonsense").id, CLASSIC);
    }
}
